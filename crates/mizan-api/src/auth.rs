use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Extension, Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Request, StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::Response,
};
use mizan_core::{AppError, AppResult, ErrorEnvelope};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{AnyPool, Row, query, query_as};
use tracing::warn;
use uuid::Uuid;

use crate::AppState;

const SESSION_TOKEN_PREFIX: &str = "mizan_sess_";
const API_KEY_PREFIX: &str = "mizan_sk_live_";
const SESSION_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;
const SESSION_HEADER: &str = "x-session-token";

type AuthHttpResult<T> = Result<T, (StatusCode, Json<ErrorEnvelope>)>;

#[derive(Debug, Clone, Serialize)]
pub struct RegisterResponse {
    pub user_id: Uuid,
    pub email: String,
    pub role: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_at: String,
    pub user_id: Uuid,
    pub role: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiKeyCreateRequest {
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyCreateResponse {
    pub id: Uuid,
    pub key: String,
    pub label: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyListItem {
    pub id: Uuid,
    pub label: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyListResponse {
    pub keys: Vec<ApiKeyListItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyRevokeResponse {
    pub revoked: bool,
    pub api_key_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct ApiKeyIdentity {
    pub api_key_id: Uuid,
    pub user_id: Uuid,
    pub user_role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyProtectedResponse {
    pub user_id: Uuid,
    pub api_key_id: Uuid,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeResponse {
    pub user_id: Uuid,
    pub role: String,
}

#[derive(Debug)]
struct DbUser {
    pub id: Uuid,
    pub password_hash: String,
    pub role: String,
}

pub async fn ensure_admin_seed(
    database: &AnyPool,
    email: &str,
    password: &str,
    role: &str,
) -> AppResult<()> {
    let email = normalize_email(email);
    if email.is_empty() || password.is_empty() {
        return Err(AppError::invalid_config(
            "admin seed",
            "MIZAN_ADMIN_EMAIL and MIZAN_ADMIN_PASSWORD must be non-empty",
        ));
    }

    if let Some(user) = find_user_by_email(database, &email).await? {
        if user.role != role {
            query("UPDATE users SET role = ? WHERE id = ?")
                .bind(role)
                .bind(user.id.to_string())
                .execute(database)
                .await
                .map_err(|error| {
                    AppError::infrastructure(format!("cannot update seeded admin role: {error}"))
                })?;
        }
        return Ok(());
    }

    create_user(database, &email, password, role).await?;
    Ok(())
}

pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> AuthHttpResult<Json<RegisterResponse>> {
    let email = normalize_email(&payload.email);
    let password = payload.password;

    if email.is_empty() || password.is_empty() {
        return Err(map_error(
            StatusCode::BAD_REQUEST,
            AppError::invalid_config("auth", "email and password are required"),
        ));
    }

    let user = create_user(&state.database, &email, &password, "member")
        .await
        .map_err(from_app_error)?;

    Ok(Json(user))
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> AuthHttpResult<Json<LoginResponse>> {
    let email = normalize_email(&payload.email);
    let password = payload.password;

    if email.is_empty() || password.is_empty() {
        return Err(map_error(
            StatusCode::BAD_REQUEST,
            AppError::invalid_config("auth", "email and password are required"),
        ));
    }

    let user = find_user_by_email(&state.database, &email)
        .await
        .map_err(from_app_error)?
        .ok_or_else(|| map_error(StatusCode::UNAUTHORIZED, AppError::Unauthorized))?;

    let verified = bcrypt::verify(&password, &user.password_hash).map_err(|error| {
        map_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            AppError::infrastructure(format!("password verification failed: {error}")),
        )
    })?;

    if !verified {
        return Err(map_error(StatusCode::UNAUTHORIZED, AppError::Unauthorized));
    }

    let session = create_session(&state.database, user.id)
        .await
        .map_err(from_app_error)?;
    Ok(Json(LoginResponse {
        access_token: session.token,
        token_type: "Bearer",
        expires_at: session.expires_at,
        user_id: user.id,
        role: user.role,
    }))
}

pub async fn create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ApiKeyCreateRequest>,
) -> AuthHttpResult<Json<ApiKeyCreateResponse>> {
    let user_id = session_user_id_from_header(&state.database, &headers).await?;
    let response = create_api_key_for_user(&state.database, user_id, payload.label)
        .await
        .map_err(from_app_error)?;
    Ok(Json(response))
}

pub async fn list_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AuthHttpResult<Json<ApiKeyListResponse>> {
    let user_id = session_user_id_from_header(&state.database, &headers).await?;
    let keys = list_api_keys_for_user(&state.database, user_id)
        .await
        .map_err(from_app_error)?;
    Ok(Json(ApiKeyListResponse { keys }))
}

pub async fn revoke_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(api_key_id): Path<Uuid>,
) -> AuthHttpResult<Json<ApiKeyRevokeResponse>> {
    let user_id = session_user_id_from_header(&state.database, &headers).await?;
    let revoked = revoke_api_key_by_owner(&state.database, user_id, api_key_id)
        .await
        .map_err(from_app_error)?;

    Ok(Json(ApiKeyRevokeResponse {
        revoked,
        api_key_id,
    }))
}

pub async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AuthHttpResult<Json<MeResponse>> {
    let user_id = session_user_id_from_header(&state.database, &headers).await?;
    let role = find_user_role_by_id(&state.database, user_id)
        .await
        .map_err(from_app_error)?;
    Ok(Json(MeResponse { user_id, role }))
}

pub async fn api_key_ping(
    Extension(identity): Extension<ApiKeyIdentity>,
) -> Json<ApiKeyProtectedResponse> {
    Json(ApiKeyProtectedResponse {
        user_id: identity.user_id,
        api_key_id: identity.api_key_id,
        role: identity.user_role,
    })
}

pub async fn api_key_auth(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> AuthHttpResult<Response> {
    let authorization = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| map_error(StatusCode::UNAUTHORIZED, AppError::Unauthorized))?;

    let identity = resolve_api_key_identity(&state.database, authorization).await?;
    request.extensions_mut().insert(identity);
    Ok(next.run(request).await)
}

async fn create_user(
    database: &AnyPool,
    email: &str,
    password: &str,
    role: &str,
) -> AppResult<RegisterResponse> {
    let hashed = bcrypt::hash(password, bcrypt::DEFAULT_COST)
        .map_err(|error| AppError::infrastructure(format!("password hash failed: {error}")))?;
    let now = unix_timestamp_string();
    let id = Uuid::now_v7();

    let affected = query(
        "INSERT INTO users (id, email, password_hash, role, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(email)
    .bind(hashed)
    .bind(role)
    .bind(&now)
    .bind(&now)
    .execute(database)
    .await
    .map_err(|error| {
        let message = error.to_string();
        if is_unique_constraint_error(&message) {
            AppError::invalid_config("email", "email is already registered")
        } else {
            AppError::infrastructure(message)
        }
    })?;

    if affected.rows_affected() != 1 {
        return Err(AppError::infrastructure("unable to insert user"));
    }

    Ok(RegisterResponse {
        user_id: id,
        email: email.to_string(),
        role: role.to_string(),
    })
}

async fn find_user_by_email(database: &AnyPool, email: &str) -> AppResult<Option<DbUser>> {
    let row = query("SELECT id, password_hash, role FROM users WHERE email = ?")
        .bind(email)
        .fetch_optional(database)
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?;

    let Some(row) = row else {
        return Ok(None);
    };

    let user_id = Uuid::parse_str(&row.try_get::<String, _>("id").map_err(|error| {
        AppError::infrastructure(format!("invalid user id in users table: {error}"))
    })?)
    .map_err(|error| {
        AppError::infrastructure(format!("invalid user id in users table: {error}"))
    })?;
    let password_hash = row
        .try_get::<String, _>("password_hash")
        .map_err(|error| AppError::infrastructure(error.to_string()))?;
    let role = row
        .try_get::<String, _>("role")
        .map_err(|error| AppError::infrastructure(error.to_string()))?;

    Ok(Some(DbUser {
        id: user_id,
        password_hash,
        role,
    }))
}

async fn find_user_role_by_id(database: &AnyPool, user_id: Uuid) -> AppResult<String> {
    let role = query_as::<_, (String,)>("SELECT role FROM users WHERE id = ?")
        .bind(user_id.to_string())
        .fetch_optional(database)
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?
        .map(|row| row.0)
        .ok_or_else(|| AppError::NotFound("user not found".to_string()))?;

    Ok(role)
}

async fn create_session(database: &AnyPool, user_id: Uuid) -> AppResult<SessionRecord> {
    let id = Uuid::now_v7();
    let token = format!("{}{}", SESSION_TOKEN_PREFIX, id);
    let token_hash = hash_value(&token);
    let now = now_utc_epoch_seconds();
    let expires_at = (now + SESSION_TTL_SECONDS).to_string();
    let timestamp = now.to_string();

    let result = query(
        "INSERT INTO sessions (
             id,
             user_id,
             session_token_hash,
             expires_at,
             revoked,
             created_at,
             updated_at
         )
         VALUES (?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(id.to_string())
    .bind(user_id.to_string())
    .bind(token_hash)
    .bind(&expires_at)
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    if result.rows_affected() != 1 {
        return Err(AppError::infrastructure("unable to create session"));
    }

    Ok(SessionRecord { token, expires_at })
}

async fn create_api_key_for_user(
    database: &AnyPool,
    user_id: Uuid,
    label: Option<String>,
) -> AppResult<ApiKeyCreateResponse> {
    let id = Uuid::now_v7();
    let key = format!("{}{}", API_KEY_PREFIX, id);
    let key_hash = hash_value(&key);
    let now = unix_timestamp_string();

    let result = query(
        "INSERT INTO api_keys (id, user_id, key_hash, label, revoked, created_at, updated_at)
         VALUES (?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(id.to_string())
    .bind(user_id.to_string())
    .bind(key_hash)
    .bind(label.clone())
    .bind(&now)
    .bind(&now)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    if result.rows_affected() != 1 {
        return Err(AppError::infrastructure("unable to create API key"));
    }

    Ok(ApiKeyCreateResponse {
        id,
        key,
        label,
        created_at: now,
    })
}

async fn list_api_keys_for_user(
    database: &AnyPool,
    user_id: Uuid,
) -> AppResult<Vec<ApiKeyListItem>> {
    let rows = query_as::<_, (String, Option<String>, String)>(
        "SELECT id, label, created_at FROM api_keys WHERE user_id = ? AND revoked = 0 ORDER BY created_at DESC",
    )
        .bind(user_id.to_string())
        .fetch_all(database)
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?;

    rows.into_iter()
        .map(|(id, label, created_at)| {
            let id = Uuid::parse_str(&id).map_err(|error| {
                AppError::infrastructure(format!("invalid api key id: {error}"))
            })?;
            Ok(ApiKeyListItem {
                id,
                label,
                created_at,
            })
        })
        .collect()
}

async fn revoke_api_key_by_owner(
    database: &AnyPool,
    user_id: Uuid,
    api_key_id: Uuid,
) -> AppResult<bool> {
    let result = query(
        "UPDATE api_keys
         SET revoked = 1, updated_at = ?
         WHERE id = ? AND user_id = ? AND revoked = 0",
    )
    .bind(unix_timestamp_string())
    .bind(api_key_id.to_string())
    .bind(user_id.to_string())
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    Ok(result.rows_affected() == 1)
}

async fn resolve_api_key_identity(
    database: &AnyPool,
    authorization: &str,
) -> AuthHttpResult<ApiKeyIdentity> {
    let token = authorization
        .trim()
        .strip_prefix("Bearer ")
        .ok_or_else(|| map_error(StatusCode::UNAUTHORIZED, AppError::Unauthorized))?;

    let token_hash = hash_value(token);
    let (api_key_id, user_id, user_role) = query_as::<_, (String, String, String)>(
        "SELECT ak.id, ak.user_id, COALESCE(u.role, 'member') AS user_role
         FROM api_keys ak
         LEFT JOIN users u ON u.id = ak.user_id
         WHERE ak.key_hash = ? AND ak.revoked = 0",
    )
    .bind(token_hash)
    .fetch_optional(database)
    .await
    .map_err(|error| {
        map_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            AppError::infrastructure(error.to_string()),
        )
    })?
    .ok_or_else(|| map_error(StatusCode::UNAUTHORIZED, AppError::Unauthorized))?;

    let api_key_id = Uuid::parse_str(&api_key_id).map_err(|error| {
        map_error(
            StatusCode::UNAUTHORIZED,
            AppError::invalid_config("api key identity", error.to_string()),
        )
    })?;
    let user_id = Uuid::parse_str(&user_id).map_err(|error| {
        map_error(
            StatusCode::UNAUTHORIZED,
            AppError::invalid_config("api key identity", error.to_string()),
        )
    })?;

    Ok(ApiKeyIdentity {
        api_key_id,
        user_id,
        user_role,
    })
}

async fn session_user_id_from_header(
    database: &AnyPool,
    headers: &HeaderMap,
) -> AuthHttpResult<Uuid> {
    let raw_token = headers
        .get(SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| map_error(StatusCode::UNAUTHORIZED, AppError::Unauthorized))?;

    let token_hash = hash_value(raw_token);
    let row = query(
        "SELECT id, user_id, expires_at
         FROM sessions
         WHERE session_token_hash = ? AND revoked = 0",
    )
    .bind(token_hash)
    .fetch_optional(database)
    .await
    .map_err(|error| {
        map_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            AppError::infrastructure(error.to_string()),
        )
    })?
    .ok_or_else(|| map_error(StatusCode::UNAUTHORIZED, AppError::Unauthorized))?;

    let session_id = row.try_get::<String, _>("id").map_err(|error| {
        map_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            AppError::infrastructure(format!("invalid session row id: {error}")),
        )
    })?;
    let user_id = row.try_get::<String, _>("user_id").map_err(|error| {
        map_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            AppError::infrastructure(format!("invalid session user id: {error}")),
        )
    })?;
    let expires_at = row
        .try_get::<Option<String>, _>("expires_at")
        .ok()
        .and_then(|raw| raw.and_then(|value| value.parse::<i64>().ok()));

    let now = now_utc_epoch_seconds();
    if let Some(expiry) = expires_at {
        if expiry <= now {
            warn!(
                session_id = %session_id,
                user_id = %user_id,
                "session expired, revoking"
            );
            query("UPDATE sessions SET revoked = 1 WHERE id = ?")
                .bind(&session_id)
                .execute(database)
                .await
                .ok();
            return Err(map_error(StatusCode::UNAUTHORIZED, AppError::Unauthorized));
        }
    } else {
        return Err(map_error(
            StatusCode::UNAUTHORIZED,
            AppError::invalid_config("session", "missing or invalid session expiry"),
        ));
    }

    Uuid::parse_str(&user_id).map_err(|error| {
        map_error(
            StatusCode::UNAUTHORIZED,
            AppError::invalid_config("session", error.to_string()),
        )
    })
}

fn map_error(status: StatusCode, error: AppError) -> (StatusCode, Json<ErrorEnvelope>) {
    (status, Json(ErrorEnvelope::from(&error)))
}

fn from_app_error(error: AppError) -> (StatusCode, Json<ErrorEnvelope>) {
    let status = match error {
        AppError::InvalidConfig { .. } => StatusCode::BAD_REQUEST,
        AppError::NotFound(_) => StatusCode::NOT_FOUND,
        AppError::Unauthorized => StatusCode::UNAUTHORIZED,
        AppError::Forbidden => StatusCode::FORBIDDEN,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    map_error(status, error)
}

fn hash_value(value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(value.as_bytes());
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn now_utc_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

fn unix_timestamp_string() -> String {
    now_utc_epoch_seconds().to_string()
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn is_unique_constraint_error(message: &str) -> bool {
    message.contains("already exists")
        || message.contains("UNIQUE constraint failed")
        || message.contains("duplicate key")
}

struct SessionRecord {
    token: String,
    expires_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_value_is_stable() {
        let first = hash_value("seed");
        let second = hash_value("seed");
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn normalize_email_is_lowercase_and_trimmed() {
        assert_eq!(normalize_email("  User@Example.COM "), "user@example.com");
    }
}
