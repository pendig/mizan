use axum::Json;
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::http::{Request, StatusCode, header::AUTHORIZATION};
use axum::middleware::Next;
use axum::response::Response;
use mizan_core::{AppError, DatabaseBackend, ErrorEnvelope};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{AnyPool, query, query_as};
use tracing::{Instrument, info_span, warn};
use uuid::Uuid;

use crate::AppState;
use crate::auth::ApiKeyIdentity;
use crate::logging::{AdminAuditInput, record_admin_audit, serialize_payload};
use crate::utils::{
    from_app_error, is_enabled, is_unique_constraint_error, prepare_sql, unix_timestamp_string,
};

type DaemonNodeHttpResult<T> = Result<T, (StatusCode, Json<ErrorEnvelope>)>;

const DAEMON_TOKEN_PREFIX: &str = "mizan_sk_daemon_";
const STATUS_PENDING: &str = "pending";
const STATUS_ACTIVE: &str = "active";
const STATUS_REVOKED: &str = "revoked";
const AUDIT_ACTION_CREATE_DAEMON_NODE: &str = "daemon_node_created";
const AUDIT_ACTION_REVOKE_DAEMON_NODE: &str = "daemon_node_revoked";
const AUDIT_ENTITY_DAEMON_NODE: &str = "daemon_node";

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonNodeCreateRequest {
    pub host_user_id: Option<Uuid>,
    pub label: Option<String>,
    pub hostname: Option<String>,
    pub public_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonNodeCreateResponse {
    pub id: Uuid,
    pub token: String,
    pub token_type: &'static str,
    pub status: String,
    pub host_user_id: Option<Uuid>,
    pub label: Option<String>,
    pub hostname: Option<String>,
    pub public_key: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonNodeResponse {
    pub id: Uuid,
    pub host_user_id: Option<Uuid>,
    pub label: Option<String>,
    pub hostname: Option<String>,
    pub public_key: Option<String>,
    pub status: String,
    pub revoked: bool,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonNodeListResponse {
    pub data: Vec<DaemonNodeResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonNodeRevokeResponse {
    pub id: Uuid,
    pub revoked: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonRegistrationRequest {
    pub hostname: Option<String>,
    pub public_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonRegistrationResponse {
    pub node_id: Uuid,
    pub status: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonPingResponse {
    pub node_id: Uuid,
    pub status: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone)]
pub struct DaemonNodeIdentity {
    pub node_id: Uuid,
    pub status: String,
}

#[derive(Debug)]
struct DbDaemonNode {
    id: String,
    host_user_id: Option<String>,
    label: Option<String>,
    hostname: Option<String>,
    public_key: Option<String>,
    status: String,
    revoked: i64,
    last_seen_at: Option<String>,
    created_at: String,
    updated_at: String,
}

pub async fn list_daemon_nodes(
    State(state): State<AppState>,
) -> DaemonNodeHttpResult<Json<DaemonNodeListResponse>> {
    let rows = query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            i64,
            Option<String>,
            String,
            String,
        ),
    >(&prepare_sql(
        state.database_backend(),
        "SELECT id,
                host_user_id,
                label,
                hostname,
                public_key,
                status,
                revoked,
                last_seen_at,
                created_at,
                updated_at
         FROM daemon_nodes
         ORDER BY created_at DESC",
    ))
    .fetch_all(&state.database)
    .await
    .map_err(|error| from_app_error(AppError::infrastructure(error.to_string())))?;

    let data = rows
        .into_iter()
        .map(
            |(
                id,
                host_user_id,
                label,
                hostname,
                public_key,
                status,
                revoked,
                last_seen_at,
                created_at,
                updated_at,
            )| {
                daemon_node_response(DbDaemonNode {
                    id,
                    host_user_id,
                    label,
                    hostname,
                    public_key,
                    status,
                    revoked,
                    last_seen_at,
                    created_at,
                    updated_at,
                })
            },
        )
        .collect::<Result<Vec<_>, _>>()
        .map_err(from_app_error)?;

    Ok(Json(DaemonNodeListResponse { data }))
}

pub async fn create_daemon_node(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
    Json(payload): Json<DaemonNodeCreateRequest>,
) -> DaemonNodeHttpResult<Json<DaemonNodeCreateResponse>> {
    let host_user_id = payload.host_user_id.unwrap_or(identity.user_id);
    ensure_user_exists(&state.database, state.database_backend(), host_user_id)
        .await
        .map_err(from_app_error)?;

    let label = normalize_optional(payload.label);
    let hostname = normalize_optional(payload.hostname);
    let public_key = normalize_optional(payload.public_key);
    let id = Uuid::now_v7();
    let token = format!("{}{}", DAEMON_TOKEN_PREFIX, Uuid::new_v4());
    let token_hash = hash_value(&token);
    let now = unix_timestamp_string();

    query(&prepare_sql(
        state.database_backend(),
        "INSERT INTO daemon_nodes (
             id,
             host_user_id,
             label,
             hostname,
             public_key,
             token_hash,
             status,
             revoked,
             created_at,
             updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
    ))
    .bind(id.to_string())
    .bind(host_user_id.to_string())
    .bind(label.as_deref())
    .bind(hostname.as_deref())
    .bind(public_key.as_deref())
    .bind(token_hash)
    .bind(STATUS_PENDING)
    .bind(&now)
    .bind(&now)
    .execute(&state.database)
    .await
    .map_err(|error| from_app_error(map_insert_error(error.to_string())))?;

    let audit = AdminAuditInput {
        actor_user_id: Some(identity.user_id),
        action: AUDIT_ACTION_CREATE_DAEMON_NODE.to_owned(),
        entity_type: AUDIT_ENTITY_DAEMON_NODE.to_owned(),
        entity_id: Some(id.to_string()),
        payload_json: serialize_payload(json!({
            "host_user_id": host_user_id.to_string(),
            "label": label,
            "hostname": hostname,
            "public_key_present": public_key.is_some(),
            "raw_secret_returned_once": true,
        })),
    };
    if let Err(error) = record_admin_audit(&state.database, state.database_backend(), &audit).await
    {
        warn!(error = %error, "failed to record daemon node creation audit");
    }

    Ok(Json(DaemonNodeCreateResponse {
        id,
        token,
        token_type: "Bearer",
        status: STATUS_PENDING.to_owned(),
        host_user_id: Some(host_user_id),
        label,
        hostname,
        public_key,
        created_at: now,
    }))
}

pub async fn revoke_daemon_node(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
    Path(id): Path<Uuid>,
) -> DaemonNodeHttpResult<Json<DaemonNodeRevokeResponse>> {
    let revoked = revoke_node(&state.database, state.database_backend(), id)
        .await
        .map_err(from_app_error)?;

    if !revoked {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorEnvelope::from(&AppError::NotFound(
                "daemon node not found".to_string(),
            ))),
        ));
    }

    let audit = AdminAuditInput {
        actor_user_id: Some(identity.user_id),
        action: AUDIT_ACTION_REVOKE_DAEMON_NODE.to_owned(),
        entity_type: AUDIT_ENTITY_DAEMON_NODE.to_owned(),
        entity_id: Some(id.to_string()),
        payload_json: serialize_payload(json!({ "revoked": true })),
    };
    if let Err(error) = record_admin_audit(&state.database, state.database_backend(), &audit).await
    {
        warn!(error = %error, "failed to record daemon node revocation audit");
    }

    Ok(Json(DaemonNodeRevokeResponse { id, revoked }))
}

pub async fn register_daemon_node(
    State(state): State<AppState>,
    Extension(identity): Extension<DaemonNodeIdentity>,
    Json(payload): Json<DaemonRegistrationRequest>,
) -> DaemonNodeHttpResult<Json<DaemonRegistrationResponse>> {
    let last_seen_at = mark_node_seen(
        &state.database,
        state.database_backend(),
        identity.node_id,
        normalize_optional(payload.hostname),
        normalize_optional(payload.public_key),
    )
    .await
    .map_err(from_app_error)?;

    Ok(Json(DaemonRegistrationResponse {
        node_id: identity.node_id,
        status: STATUS_ACTIVE.to_owned(),
        last_seen_at,
    }))
}

pub async fn daemon_ping(
    State(state): State<AppState>,
    Extension(identity): Extension<DaemonNodeIdentity>,
) -> DaemonNodeHttpResult<Json<DaemonPingResponse>> {
    let last_seen_at = mark_node_seen(
        &state.database,
        state.database_backend(),
        identity.node_id,
        None,
        None,
    )
    .await
    .map_err(from_app_error)?;

    Ok(Json(DaemonPingResponse {
        node_id: identity.node_id,
        status: identity.status,
        last_seen_at,
    }))
}

pub async fn daemon_node_auth(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> DaemonNodeHttpResult<Response> {
    let authorization = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| map_error(StatusCode::UNAUTHORIZED, AppError::Unauthorized))?;

    let identity =
        resolve_daemon_node_identity(&state.database, state.database_backend(), authorization)
            .instrument(info_span!("daemon_node_auth"))
            .await?;
    request.extensions_mut().insert(identity);
    Ok(next.run(request).await)
}

async fn ensure_user_exists(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    user_id: Uuid,
) -> Result<(), AppError> {
    let exists = query_as::<_, (i64,)>(&prepare_sql(
        database_backend,
        "SELECT 1 FROM users WHERE id = ?",
    ))
    .bind(user_id.to_string())
    .fetch_optional(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    if exists.is_none() {
        return Err(AppError::invalid_config(
            "daemon_node.host_user_id",
            "host_user_id does not exist",
        ));
    }

    Ok(())
}

async fn resolve_daemon_node_identity(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    authorization: &str,
) -> DaemonNodeHttpResult<DaemonNodeIdentity> {
    let token = authorization_token(authorization)
        .ok_or_else(|| map_error(StatusCode::UNAUTHORIZED, AppError::Unauthorized))?;
    let token_hash = hash_value(token);

    let row = query_as::<_, (String, String)>(&prepare_sql(
        database_backend,
        "SELECT id, status
         FROM daemon_nodes
         WHERE token_hash = ? AND revoked = 0 AND status != ?",
    ))
    .bind(token_hash)
    .bind(STATUS_REVOKED)
    .fetch_optional(database)
    .await
    .map_err(|error| {
        map_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            AppError::infrastructure(error.to_string()),
        )
    })?
    .ok_or_else(|| map_error(StatusCode::UNAUTHORIZED, AppError::Unauthorized))?;

    let node_id = Uuid::parse_str(&row.0).map_err(|error| {
        map_error(
            StatusCode::UNAUTHORIZED,
            AppError::invalid_config("daemon node identity", error.to_string()),
        )
    })?;

    Ok(DaemonNodeIdentity {
        node_id,
        status: row.1,
    })
}

async fn mark_node_seen(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    node_id: Uuid,
    hostname: Option<String>,
    public_key: Option<String>,
) -> Result<String, AppError> {
    let now = unix_timestamp_string();

    let result = query(&prepare_sql(
        database_backend,
        "UPDATE daemon_nodes
         SET status = ?,
             last_seen_at = ?,
             hostname = COALESCE(?, hostname),
             public_key = COALESCE(?, public_key),
             updated_at = ?
         WHERE id = ? AND revoked = 0 AND status != ?",
    ))
    .bind(STATUS_ACTIVE)
    .bind(&now)
    .bind(hostname.as_deref())
    .bind(public_key.as_deref())
    .bind(&now)
    .bind(node_id.to_string())
    .bind(STATUS_REVOKED)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    if result.rows_affected() != 1 {
        return Err(AppError::Unauthorized);
    }

    Ok(now)
}

async fn revoke_node(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    node_id: Uuid,
) -> Result<bool, AppError> {
    let now = unix_timestamp_string();
    let result = query(&prepare_sql(
        database_backend,
        "UPDATE daemon_nodes
         SET status = ?, revoked = 1, updated_at = ?
         WHERE id = ? AND revoked = 0",
    ))
    .bind(STATUS_REVOKED)
    .bind(&now)
    .bind(node_id.to_string())
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    Ok(result.rows_affected() == 1)
}

fn daemon_node_response(row: DbDaemonNode) -> Result<DaemonNodeResponse, AppError> {
    let id = Uuid::parse_str(&row.id)
        .map_err(|error| AppError::infrastructure(format!("invalid daemon node id: {error}")))?;
    let host_user_id = row
        .host_user_id
        .as_deref()
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|error| {
            AppError::infrastructure(format!("invalid daemon node host user id: {error}"))
        })?;

    Ok(DaemonNodeResponse {
        id,
        host_user_id,
        label: row.label,
        hostname: row.hostname,
        public_key: row.public_key,
        status: row.status,
        revoked: is_enabled(row.revoked),
        last_seen_at: row.last_seen_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn authorization_token(raw_authorization: &str) -> Option<&str> {
    let mut split = raw_authorization.split_whitespace();
    let scheme = split.next()?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }

    split.next()
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

fn map_error(status: StatusCode, error: AppError) -> (StatusCode, Json<ErrorEnvelope>) {
    (status, Json(ErrorEnvelope::from(&error)))
}

fn map_insert_error(error: String) -> AppError {
    if is_unique_constraint_error(&error) {
        AppError::invalid_config("daemon_node.token", "daemon node token must be unique")
    } else {
        AppError::infrastructure(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;
    use sqlx::query_scalar;

    async fn sqlite_test_database() -> AnyPool {
        storage::connect_and_migrate("sqlite::memory:", true, 1)
            .await
            .expect("create sqlite test database")
    }

    async fn seed_user(database: &AnyPool) -> Uuid {
        let id = Uuid::now_v7();
        let now = unix_timestamp_string();
        query(&prepare_sql(
            DatabaseBackend::Sqlite,
            "INSERT INTO users (id, email, password_hash, role, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        ))
        .bind(id.to_string())
        .bind(format!("{id}@example.test"))
        .bind("hash")
        .bind("admin")
        .bind(&now)
        .bind(&now)
        .execute(database)
        .await
        .expect("insert user");
        id
    }

    async fn insert_node(database: &AnyPool, token: &str, revoked: bool) -> Uuid {
        let node_id = Uuid::now_v7();
        let user_id = seed_user(database).await;
        let now = unix_timestamp_string();
        query(&prepare_sql(
            DatabaseBackend::Sqlite,
            "INSERT INTO daemon_nodes (
                 id, host_user_id, token_hash, status, revoked, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        ))
        .bind(node_id.to_string())
        .bind(user_id.to_string())
        .bind(hash_value(token))
        .bind(if revoked {
            STATUS_REVOKED
        } else {
            STATUS_PENDING
        })
        .bind(if revoked { 1 } else { 0 })
        .bind(&now)
        .bind(&now)
        .execute(database)
        .await
        .expect("insert daemon node");
        node_id
    }

    #[tokio::test]
    async fn daemon_registration_accepts_valid_token_and_marks_node_seen() {
        let database = sqlite_test_database().await;
        let token = "mizan_sk_daemon_valid";
        let node_id = insert_node(&database, token, false).await;

        let identity = resolve_daemon_node_identity(
            &database,
            DatabaseBackend::Sqlite,
            &format!("Bearer {token}"),
        )
        .await
        .expect("valid daemon token should authenticate");
        assert_eq!(identity.node_id, node_id);

        let last_seen = mark_node_seen(
            &database,
            DatabaseBackend::Sqlite,
            node_id,
            Some("host-a".to_owned()),
            Some("ssh-ed25519 test".to_owned()),
        )
        .await
        .expect("mark seen");
        assert!(!last_seen.is_empty());

        let status: String = query_scalar("SELECT status FROM daemon_nodes WHERE id = ?")
            .bind(node_id.to_string())
            .fetch_one(&database)
            .await
            .expect("read status");
        assert_eq!(status, STATUS_ACTIVE);
    }

    #[tokio::test]
    async fn daemon_registration_rejects_invalid_token() {
        let database = sqlite_test_database().await;
        insert_node(&database, "mizan_sk_daemon_valid", false).await;

        let error = resolve_daemon_node_identity(
            &database,
            DatabaseBackend::Sqlite,
            "Bearer mizan_sk_daemon_invalid",
        )
        .await
        .expect_err("invalid token should fail");

        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn daemon_registration_rejects_revoked_node() {
        let database = sqlite_test_database().await;
        let token = "mizan_sk_daemon_revoked";
        insert_node(&database, token, true).await;

        let error = resolve_daemon_node_identity(
            &database,
            DatabaseBackend::Sqlite,
            &format!("Bearer {token}"),
        )
        .await
        .expect_err("revoked node should fail");

        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
    }
}
