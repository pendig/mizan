use std::collections::HashSet;

use axum::Json;
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use mizan_core::{AppError, DatabaseBackend, ErrorEnvelope};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{AnyPool, query, query_as};
use tracing::warn;
use uuid::Uuid;

use crate::AppState;
use crate::auth::ApiKeyIdentity;
use crate::daemon_nodes::{HEALTH_STATUS_HEALTHY, STATUS_ACTIVE, parse_json_vec};
use crate::logging::{AdminAuditInput, record_admin_audit, serialize_payload};
use crate::utils::{
    encrypt_provider_api_key, from_app_error, is_enabled, is_unique_constraint_error,
    now_utc_epoch_seconds, parse_timestamp, prepare_sql, unix_timestamp_string,
};

type ProviderHttpResult<T> = Result<T, (StatusCode, Json<ErrorEnvelope>)>;
const AUDIT_ACTION_CREATE_PROVIDER: &str = "provider_connection_created";
const AUDIT_ACTION_DELETE_PROVIDER: &str = "provider_connection_deleted";
const AUDIT_ACTION_CREATE_MODEL_ROUTE: &str = "model_route_created";
const AUDIT_ACTION_DELETE_MODEL_ROUTE: &str = "model_route_deleted";
const AUDIT_ENTITY_PROVIDER: &str = "provider_connection";
const AUDIT_ENTITY_MODEL_ROUTE: &str = "model_route";
const AUTH_MODE_API_KEY: &str = "api_key";
const AUTH_MODE_DAEMON: &str = "daemon";
const AUTH_MODE_SUBSCRIPTION_CLI: &str = "subscription_cli";
const AUTH_MODE_BROWSER_SESSION: &str = "browser_session";

#[derive(Debug, Serialize)]
pub struct ProviderConnectionResponse {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub auth_mode: String,
    pub base_url: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ProviderConnectionListResponse {
    pub data: Vec<ProviderConnectionResponse>,
}

#[derive(Debug, Serialize)]
pub struct ProviderConnectionCreateResponse {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub auth_mode: String,
    pub base_url: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct ProviderConnectionWithStatus {
    pub id: String,
    pub removed: bool,
}

#[derive(Debug, Deserialize)]
pub struct ProviderConnectionCreateRequest {
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<String>,
    pub auth_mode: Option<String>,
    pub auth_config_json: Option<Value>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ModelRouteResponse {
    pub id: String,
    pub provider_connection_id: String,
    pub public_model: String,
    pub upstream_model: String,
    pub max_tokens: Option<i64>,
    pub pricing_input_per_1m_tokens: i64,
    pub pricing_output_per_1m_tokens: i64,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    pub provider_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ModelRouteListResponse {
    pub data: Vec<ModelRouteResponse>,
}

#[derive(Debug, Serialize)]
pub struct ModelRouteCreateResponse {
    pub id: String,
    pub provider_connection_id: String,
    pub public_model: String,
    pub upstream_model: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct ModelRouteWithStatus {
    pub id: String,
    pub removed: bool,
}

#[derive(Debug, Deserialize)]
pub struct ModelRouteCreateRequest {
    pub provider_connection_id: Uuid,
    pub public_model: String,
    pub upstream_model: String,
    pub max_tokens: Option<i64>,
    pub pricing_input_per_1m_tokens: Option<i64>,
    pub pricing_output_per_1m_tokens: Option<i64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct PublicModelResponse {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub owned_by: String,
    pub provider_type: String,
    pub upstream_model: String,
    pub route_id: String,
    pub max_tokens: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PublicModelsResponse {
    pub object: &'static str,
    pub data: Vec<PublicModelResponse>,
}

pub async fn require_admin_role(
    identity: Option<axum::Extension<ApiKeyIdentity>>,
    request: axum::http::Request<Body>,
    next: Next,
) -> ProviderHttpResult<Response> {
    let identity = identity.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorEnvelope::from(&AppError::Unauthorized)),
        )
    })?;
    let identity = identity.0;

    if identity.user_role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorEnvelope::from(&AppError::Forbidden)),
        ));
    }

    Ok(next.run(request).await)
}

fn normalize_auth_mode(raw_mode: Option<&str>) -> Result<&'static str, AppError> {
    let mode = raw_mode
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(AUTH_MODE_API_KEY);

    match mode {
        AUTH_MODE_API_KEY => Ok(AUTH_MODE_API_KEY),
        AUTH_MODE_DAEMON => Ok(AUTH_MODE_DAEMON),
        AUTH_MODE_SUBSCRIPTION_CLI => Ok(AUTH_MODE_SUBSCRIPTION_CLI),
        AUTH_MODE_BROWSER_SESSION => Ok(AUTH_MODE_BROWSER_SESSION),
        _ => Err(AppError::invalid_config(
            "provider_connection.auth_mode",
            "auth_mode must be api_key, daemon, subscription_cli, or browser_session",
        )),
    }
}

fn normalize_auth_config(
    auth_mode: &str,
    raw_config: Option<&Value>,
) -> Result<Option<String>, AppError> {
    if auth_mode == AUTH_MODE_API_KEY {
        return Ok(None);
    }

    let Some(config) = raw_config else {
        return Ok(None);
    };

    let Some(config_object) = config.as_object() else {
        return Err(AppError::invalid_config(
            "provider_connection.auth_config_json",
            "auth_config_json must be a JSON object",
        ));
    };

    for forbidden_key in ["api_key", "access_token", "refresh_token", "password"] {
        if config_object.contains_key(forbidden_key) {
            return Err(AppError::invalid_config(
                "provider_connection.auth_config_json",
                "auth_config_json must store references or non-secret metadata, not raw secrets",
            ));
        }
    }

    serde_json::to_string(config).map(Some).map_err(|error| {
        AppError::invalid_config(
            "provider_connection.auth_config_json",
            format!("auth_config_json is not serializable: {error}"),
        )
    })
}

pub async fn list_models(
    State(state): State<AppState>,
) -> ProviderHttpResult<Json<PublicModelsResponse>> {
    let data = list_public_models(
        &state.database,
        state.database_backend(),
        i64::from(state.config.daemon_stale_seconds),
    )
    .await
    .map_err(from_app_error)?;

    Ok(Json(PublicModelsResponse {
        object: "list",
        data,
    }))
}

async fn list_public_models(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    daemon_stale_seconds: i64,
) -> Result<Vec<PublicModelResponse>, AppError> {
    let rows =
        query_as::<_, (String, String, String, String, String, Option<i64>, String)>(&prepare_sql(
            database_backend,
            "SELECT mr.public_model,
                    mr.upstream_model,
                    mr.id,
                    pc.name,
                    pc.provider_type,
                    mr.max_tokens,
             mr.created_at
             FROM model_routes mr
             INNER JOIN provider_connections pc
               ON pc.id = mr.provider_connection_id
             WHERE mr.enabled = ? AND pc.enabled = ?
             ORDER BY mr.public_model ASC",
        ))
        .bind(1)
        .bind(1)
        .fetch_all(database)
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?;

    let mut data = Vec::with_capacity(rows.len());
    let mut seen_model_ids = HashSet::new();

    for (
        public_model,
        upstream_model,
        route_id,
        provider_name,
        provider_type,
        max_tokens,
        created_at,
    ) in rows
    {
        let created = parse_timestamp(&created_at)?;
        seen_model_ids.insert(public_model.clone());

        data.push(PublicModelResponse {
            id: public_model.clone(),
            object: "model",
            created,
            owned_by: provider_name,
            provider_type,
            upstream_model,
            route_id,
            max_tokens,
        });
    }

    let cutoff = now_utc_epoch_seconds().saturating_sub(daemon_stale_seconds.max(1));
    let daemon_rows = query_as::<_, (String, String)>(&prepare_sql(
        database_backend,
        "SELECT provider_family, model_ids_json
         FROM daemon_nodes
         WHERE status = ?
           AND revoked = 0
           AND disabled = 0
           AND health_status = ?
           AND provider_family IS NOT NULL
           AND max_concurrency IS NOT NULL
           AND max_concurrency > 0
           AND last_seen_at IS NOT NULL
           AND last_seen_at >= ?
         ORDER BY provider_family ASC, model_ids_json ASC",
    ))
    .bind(STATUS_ACTIVE)
    .bind(HEALTH_STATUS_HEALTHY)
    .bind(cutoff.to_string())
    .fetch_all(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    for (provider_family, model_ids_json) in daemon_rows {
        let model_ids = match parse_json_vec(&model_ids_json, "daemon_node.model_ids_json") {
            Ok(model_ids) => model_ids,
            Err(error) => {
                warn!(error = %error, "skipping daemon node with invalid model capabilities");
                continue;
            }
        };

        for model_id in model_ids {
            if !seen_model_ids.insert(model_id.clone()) {
                continue;
            }

            data.push(PublicModelResponse {
                id: model_id.clone(),
                object: "model",
                created: 0,
                owned_by: "mizan-daemon".to_owned(),
                provider_type: provider_family.clone(),
                upstream_model: model_id,
                route_id: "daemon".to_owned(),
                max_tokens: None,
            });
        }
    }

    data.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(data)
}

pub async fn list_provider_connections(
    State(state): State<AppState>,
) -> ProviderHttpResult<Json<ProviderConnectionListResponse>> {
    let rows =
        query_as::<_, (String, String, String, String, String, i64, String, String)>(&prepare_sql(
            state.database_backend(),
            "SELECT id,
                    name,
                    provider_type,
                    auth_mode,
                    base_url,
                    enabled,
                    created_at,
                    updated_at
             FROM provider_connections
             ORDER BY created_at DESC",
        ))
        .fetch_all(&state.database)
        .await
        .map_err(|error| from_app_error(AppError::infrastructure(error.to_string())))?;

    let data = rows
        .into_iter()
        .map(
            |(id, name, provider_type, auth_mode, base_url, enabled, created_at, updated_at)| {
                ProviderConnectionResponse {
                    id,
                    name,
                    provider_type,
                    auth_mode,
                    base_url,
                    enabled: is_enabled(enabled),
                    created_at,
                    updated_at,
                }
            },
        )
        .collect();

    Ok(Json(ProviderConnectionListResponse { data }))
}

pub async fn create_provider_connection(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
    Json(payload): Json<ProviderConnectionCreateRequest>,
) -> ProviderHttpResult<Json<ProviderConnectionCreateResponse>> {
    let name = payload.name.trim();
    let provider_type = payload.provider_type.trim();
    let auth_mode = normalize_auth_mode(payload.auth_mode.as_deref()).map_err(from_app_error)?;
    let base_url = payload
        .base_url
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let secret = payload
        .api_key_encrypted
        .as_deref()
        .unwrap_or_default()
        .trim();
    let auth_config_json = normalize_auth_config(auth_mode, payload.auth_config_json.as_ref())
        .map_err(from_app_error)?;

    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "provider_connection.name",
                "provider name is required",
            ))),
        ));
    }

    if provider_type.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "provider_connection.provider_type",
                "provider_type is required",
            ))),
        ));
    }

    if auth_mode == AUTH_MODE_API_KEY && base_url.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "provider_connection.base_url",
                "base_url is required for api_key provider connections",
            ))),
        ));
    }

    if auth_mode == AUTH_MODE_API_KEY && secret.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "provider_connection.api_key_encrypted",
                "api_key_encrypted is required for api_key provider connections",
            ))),
        ));
    }

    let id = Uuid::now_v7();
    let now = unix_timestamp_string();
    let enabled = payload.enabled.unwrap_or(true);
    let encrypted_api_key = if auth_mode == AUTH_MODE_API_KEY {
        let provider_secret_key = state.config.provider_secret_key.as_deref().ok_or_else(|| {
            from_app_error(AppError::invalid_config(
                "MIZAN_PROVIDER_SECRET_KEY",
                "set MIZAN_PROVIDER_SECRET_KEY before creating api_key provider connections",
            ))
        })?;
        encrypt_provider_api_key(provider_secret_key, &id.to_string(), secret)
            .map_err(from_app_error)?
    } else {
        String::new()
    };

    let sql = prepare_sql(
        state.database_backend(),
        "INSERT INTO provider_connections (
             id, name, provider_type, auth_mode, auth_config_json, base_url, api_key_encrypted, enabled, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    );

    query(&sql)
        .bind(id.to_string())
        .bind(name)
        .bind(provider_type)
        .bind(auth_mode)
        .bind(auth_config_json.as_deref())
        .bind(&base_url)
        .bind(encrypted_api_key)
        .bind(if enabled { 1 } else { 0 })
        .bind(&now)
        .bind(&now)
        .execute(&state.database)
        .await
        .map_err(|error| {
            from_app_error(map_duplicate_name_error(
                error.to_string(),
                "provider connection",
            ))
        })?;

    let audit = AdminAuditInput {
        actor_user_id: Some(identity.user_id),
        action: AUDIT_ACTION_CREATE_PROVIDER.to_owned(),
        entity_type: AUDIT_ENTITY_PROVIDER.to_owned(),
        entity_id: Some(id.to_string()),
        payload_json: serialize_payload(json!({
            "name": name,
            "provider_type": provider_type,
            "auth_mode": auth_mode,
            "base_url": base_url,
            "enabled": enabled,
            "secret_material_stored": auth_mode == AUTH_MODE_API_KEY,
        })),
    };
    if let Err(error) = record_admin_audit(&state.database, state.database_backend(), &audit).await
    {
        warn!(error = %error, "failed to record provider connection creation audit");
    }

    Ok(Json(ProviderConnectionCreateResponse {
        id: id.to_string(),
        name: name.to_string(),
        provider_type: provider_type.to_string(),
        auth_mode: auth_mode.to_string(),
        base_url,
        enabled,
    }))
}

pub async fn delete_provider_connection(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
    Path(id): Path<Uuid>,
) -> ProviderHttpResult<Json<ProviderConnectionWithStatus>> {
    let removed = query(&prepare_sql(
        state.database_backend(),
        "DELETE FROM provider_connections WHERE id = ?",
    ))
    .bind(id.to_string())
    .execute(&state.database)
    .await
    .map_err(|error| from_app_error(AppError::infrastructure(error.to_string())))?;

    if removed.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorEnvelope::from(&AppError::NotFound(
                "provider connection not found".to_string(),
            ))),
        ));
    }

    let audit = AdminAuditInput {
        actor_user_id: Some(identity.user_id),
        action: AUDIT_ACTION_DELETE_PROVIDER.to_owned(),
        entity_type: AUDIT_ENTITY_PROVIDER.to_owned(),
        entity_id: Some(id.to_string()),
        payload_json: serialize_payload(json!({
            "deleted": true,
        })),
    };
    if let Err(error) = record_admin_audit(&state.database, state.database_backend(), &audit).await
    {
        warn!(error = %error, "failed to record provider connection deletion audit");
    }

    Ok(Json(ProviderConnectionWithStatus {
        id: id.to_string(),
        removed: true,
    }))
}

pub async fn list_model_routes(
    State(state): State<AppState>,
) -> ProviderHttpResult<Json<ModelRouteListResponse>> {
    let rows = query_as::<
        _,
        (
            String,
            String,
            String,
            String,
            Option<i64>,
            i64,
            i64,
            i64,
            String,
            String,
            Option<String>,
        ),
    >(&prepare_sql(
        state.database_backend(),
        "SELECT mr.id,
                    mr.provider_connection_id,
                    mr.public_model,
                    mr.upstream_model,
                    mr.max_tokens,
                    mr.pricing_input_per_1m_tokens,
                    mr.pricing_output_per_1m_tokens,
                    mr.enabled,
                    mr.created_at,
                    mr.updated_at,
                    pc.name
             FROM model_routes mr
             INNER JOIN provider_connections pc
               ON pc.id = mr.provider_connection_id
             ORDER BY mr.created_at DESC",
    ))
    .fetch_all(&state.database)
    .await
    .map_err(|error| from_app_error(AppError::infrastructure(error.to_string())))?;

    let data = rows
        .into_iter()
        .map(
            |(
                id,
                provider_connection_id,
                public_model,
                upstream_model,
                max_tokens,
                pricing_input_per_1m_tokens,
                pricing_output_per_1m_tokens,
                enabled,
                created_at,
                updated_at,
                provider_name,
            )| {
                ModelRouteResponse {
                    id,
                    provider_connection_id,
                    public_model,
                    upstream_model,
                    max_tokens,
                    pricing_input_per_1m_tokens,
                    pricing_output_per_1m_tokens,
                    enabled: is_enabled(enabled),
                    created_at,
                    updated_at,
                    provider_name,
                }
            },
        )
        .collect();

    Ok(Json(ModelRouteListResponse { data }))
}

pub async fn create_model_route(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
    Json(payload): Json<ModelRouteCreateRequest>,
) -> ProviderHttpResult<Json<ModelRouteCreateResponse>> {
    let public_model = payload.public_model.trim();
    let upstream_model = payload.upstream_model.trim();

    if public_model.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "model_route.public_model",
                "public_model is required",
            ))),
        ));
    }

    if upstream_model.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "model_route.upstream_model",
                "upstream_model is required",
            ))),
        ));
    }

    if let Some(max_tokens) = payload.max_tokens
        && max_tokens < 0
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "model_route.max_tokens",
                "max_tokens cannot be negative",
            ))),
        ));
    }

    if let Some(pricing_input_per_1m_tokens) = payload.pricing_input_per_1m_tokens
        && pricing_input_per_1m_tokens < 0
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "model_route.pricing_input_per_1m_tokens",
                "pricing_input_per_1m_tokens cannot be negative",
            ))),
        ));
    }

    if let Some(pricing_output_per_1m_tokens) = payload.pricing_output_per_1m_tokens
        && pricing_output_per_1m_tokens < 0
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "model_route.pricing_output_per_1m_tokens",
                "pricing_output_per_1m_tokens cannot be negative",
            ))),
        ));
    }

    let provider_connection_id = payload.provider_connection_id;

    let provider_exists = query_as::<_, (i64,)>(&prepare_sql(
        state.database_backend(),
        "SELECT 1 FROM provider_connections WHERE id = ? AND enabled = ?",
    ))
    .bind(provider_connection_id.to_string())
    .bind(1)
    .fetch_optional(&state.database)
    .await
    .map_err(|error| from_app_error(AppError::infrastructure(error.to_string())))?;

    if provider_exists.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "model_route.provider_connection_id",
                "provider_connection_id does not exist",
            ))),
        ));
    }

    let id = Uuid::now_v7();
    let now = unix_timestamp_string();
    let enabled = payload.enabled.unwrap_or(true);

    query(&prepare_sql(
        state.database_backend(),
        "INSERT INTO model_routes (
                id,
                provider_connection_id,
                public_model,
                upstream_model,
                max_tokens,
                pricing_input_per_1m_tokens,
                pricing_output_per_1m_tokens,
                enabled,
                created_at,
                updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(id.to_string())
    .bind(provider_connection_id.to_string())
    .bind(public_model)
    .bind(upstream_model)
    .bind(payload.max_tokens)
    .bind(payload.pricing_input_per_1m_tokens.unwrap_or(0))
    .bind(payload.pricing_output_per_1m_tokens.unwrap_or(0))
    .bind(if enabled { 1 } else { 0 })
    .bind(&now)
    .bind(&now)
    .execute(&state.database)
    .await
    .map_err(|error| from_app_error(map_duplicate_model_error(error.to_string())))?;

    let audit = AdminAuditInput {
        actor_user_id: Some(identity.user_id),
        action: AUDIT_ACTION_CREATE_MODEL_ROUTE.to_owned(),
        entity_type: AUDIT_ENTITY_MODEL_ROUTE.to_owned(),
        entity_id: Some(id.to_string()),
        payload_json: serialize_payload(json!({
            "provider_connection_id": provider_connection_id.to_string(),
            "public_model": public_model,
            "upstream_model": upstream_model,
            "max_tokens": payload.max_tokens,
            "pricing_input_per_1m_tokens": payload.pricing_input_per_1m_tokens.unwrap_or(0),
            "pricing_output_per_1m_tokens": payload.pricing_output_per_1m_tokens.unwrap_or(0),
            "enabled": enabled,
        })),
    };
    if let Err(error) = record_admin_audit(&state.database, state.database_backend(), &audit).await
    {
        warn!(error = %error, "failed to record model route creation audit");
    }

    Ok(Json(ModelRouteCreateResponse {
        id: id.to_string(),
        provider_connection_id: provider_connection_id.to_string(),
        public_model: public_model.to_string(),
        upstream_model: upstream_model.to_string(),
        enabled,
    }))
}

pub async fn delete_model_route(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
    Path(id): Path<Uuid>,
) -> ProviderHttpResult<Json<ModelRouteWithStatus>> {
    let removed = query(&prepare_sql(
        state.database_backend(),
        "DELETE FROM model_routes WHERE id = ?",
    ))
    .bind(id.to_string())
    .execute(&state.database)
    .await
    .map_err(|error| from_app_error(AppError::infrastructure(error.to_string())))?;

    if removed.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorEnvelope::from(&AppError::NotFound(
                "model route not found".to_string(),
            ))),
        ));
    }

    let audit = AdminAuditInput {
        actor_user_id: Some(identity.user_id),
        action: AUDIT_ACTION_DELETE_MODEL_ROUTE.to_owned(),
        entity_type: AUDIT_ENTITY_MODEL_ROUTE.to_owned(),
        entity_id: Some(id.to_string()),
        payload_json: serialize_payload(json!({
            "deleted": true,
        })),
    };
    if let Err(error) = record_admin_audit(&state.database, state.database_backend(), &audit).await
    {
        warn!(error = %error, "failed to record model route deletion audit");
    }

    Ok(Json(ModelRouteWithStatus {
        id: id.to_string(),
        removed: true,
    }))
}

fn map_duplicate_name_error(error: String, context: &str) -> AppError {
    if is_unique_constraint_error(&error) {
        AppError::invalid_config(
            "provider_connection.name",
            format!("{} with this name already exists", context),
        )
    } else {
        AppError::infrastructure(error)
    }
}

fn map_duplicate_model_error(error: String) -> AppError {
    if is_unique_constraint_error(&error) {
        AppError::invalid_config("model_route.public_model", "public_model must be unique")
    } else {
        AppError::infrastructure(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

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

    async fn insert_model_route(database: &AnyPool, public_model: &str) {
        let provider_id = Uuid::now_v7();
        let route_id = Uuid::now_v7();
        let now = unix_timestamp_string();
        query(&prepare_sql(
            DatabaseBackend::Sqlite,
            "INSERT INTO provider_connections (
                 id, name, provider_type, base_url, api_key_encrypted, enabled, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, 1, ?, ?)",
        ))
        .bind(provider_id.to_string())
        .bind(format!("provider-{provider_id}"))
        .bind("openai-compatible")
        .bind("http://127.0.0.1:11434/v1")
        .bind("encrypted")
        .bind(&now)
        .bind(&now)
        .execute(database)
        .await
        .expect("insert provider");

        query(&prepare_sql(
            DatabaseBackend::Sqlite,
            "INSERT INTO model_routes (
                 id,
                 provider_connection_id,
                 public_model,
                 upstream_model,
                 max_tokens,
                 pricing_input_per_1m_tokens,
                 pricing_output_per_1m_tokens,
                 enabled,
                 created_at,
                 updated_at
             ) VALUES (?, ?, ?, ?, ?, 0, 0, 1, ?, ?)",
        ))
        .bind(route_id.to_string())
        .bind(provider_id.to_string())
        .bind(public_model)
        .bind(public_model)
        .bind(None::<i64>)
        .bind(&now)
        .bind(&now)
        .execute(database)
        .await
        .expect("insert model route");
    }

    async fn insert_daemon_node(
        database: &AnyPool,
        models: Vec<&str>,
        last_seen_at: i64,
        disabled: i32,
        health_status: &str,
        metadata: Option<Value>,
    ) {
        let node_id = Uuid::now_v7();
        let user_id = seed_user(database).await;
        let now = unix_timestamp_string();
        let model_ids = models
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        query(&prepare_sql(
            DatabaseBackend::Sqlite,
            "INSERT INTO daemon_nodes (
                 id,
                 host_user_id,
                 label,
                 hostname,
                 token_hash,
                 status,
                 revoked,
                 disabled,
                 last_seen_at,
                 provider_family,
                 model_ids_json,
                 max_concurrency,
                 health_status,
                 capability_metadata_json,
                 created_at,
                 updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        ))
        .bind(node_id.to_string())
        .bind(user_id.to_string())
        .bind("private-gpu-node")
        .bind("workstation.local")
        .bind(format!("token-{node_id}"))
        .bind(STATUS_ACTIVE)
        .bind(disabled)
        .bind(last_seen_at.to_string())
        .bind("openai-compatible")
        .bind(serde_json::to_string(&model_ids).expect("serialize model ids"))
        .bind(4_i32)
        .bind(health_status)
        .bind(metadata.map(|value| value.to_string()))
        .bind(&now)
        .bind(&now)
        .execute(database)
        .await
        .expect("insert daemon node");
    }

    #[test]
    fn normalize_auth_mode_defaults_to_api_key() {
        assert_eq!(normalize_auth_mode(None).unwrap(), AUTH_MODE_API_KEY);
        assert_eq!(
            normalize_auth_mode(Some(" api_key ")).unwrap(),
            AUTH_MODE_API_KEY
        );
    }

    #[test]
    fn normalize_auth_config_rejects_raw_secret_fields() {
        let config = json!({
            "cli": "gemini",
            "access_token": "secret"
        });

        assert!(normalize_auth_config(AUTH_MODE_SUBSCRIPTION_CLI, Some(&config)).is_err());
    }

    #[test]
    fn normalize_auth_config_accepts_non_secret_reference_metadata() {
        let config = json!({
            "cli": "gemini",
            "profile": "default",
            "credential_ref": "keychain://mizan/gemini/default"
        });

        let normalized = normalize_auth_config(AUTH_MODE_SUBSCRIPTION_CLI, Some(&config))
            .expect("normalize non-secret auth config")
            .expect("non-api auth config should be stored");

        assert!(normalized.contains("credential_ref"));
    }

    #[tokio::test]
    async fn list_public_models_includes_live_daemon_capabilities_without_node_metadata() {
        let database = sqlite_test_database().await;
        let now = now_utc_epoch_seconds();

        insert_daemon_node(
            &database,
            vec!["llama3.1", "qwen2.5-coder"],
            now,
            0,
            HEALTH_STATUS_HEALTHY,
            Some(json!({"local_provider_url": "http://127.0.0.1:11434/v1"})),
        )
        .await;

        let models = list_public_models(&database, DatabaseBackend::Sqlite, 60)
            .await
            .expect("list public models");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "llama3.1");
        assert_eq!(models[0].owned_by, "mizan-daemon");
        assert_eq!(models[0].provider_type, "openai-compatible");
        assert_eq!(models[0].upstream_model, "llama3.1");
        assert_eq!(models[0].route_id, "daemon");
        let serialized = serde_json::to_string(&models).expect("serialize models");
        assert!(!serialized.contains("private-gpu-node"));
        assert!(!serialized.contains("workstation.local"));
        assert!(!serialized.contains("127.0.0.1:11434"));
    }

    #[tokio::test]
    async fn list_public_models_filters_unavailable_daemon_nodes_and_deduplicates_routes() {
        let database = sqlite_test_database().await;
        let now = now_utc_epoch_seconds();
        insert_model_route(&database, "route-backed").await;
        insert_daemon_node(
            &database,
            vec!["route-backed", "daemon-only"],
            now,
            0,
            HEALTH_STATUS_HEALTHY,
            None,
        )
        .await;
        insert_daemon_node(
            &database,
            vec!["stale-model"],
            now - 120,
            0,
            HEALTH_STATUS_HEALTHY,
            None,
        )
        .await;
        insert_daemon_node(
            &database,
            vec!["disabled-model"],
            now,
            1,
            HEALTH_STATUS_HEALTHY,
            None,
        )
        .await;
        insert_daemon_node(&database, vec!["sick-model"], now, 0, "degraded", None).await;

        let models = list_public_models(&database, DatabaseBackend::Sqlite, 60)
            .await
            .expect("list public models");
        let ids = models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["daemon-only", "route-backed"]);
    }

    #[tokio::test]
    async fn list_public_models_skips_daemon_nodes_with_invalid_model_capabilities() {
        let database = sqlite_test_database().await;
        let now = now_utc_epoch_seconds();

        insert_daemon_node(
            &database,
            vec!["healthy-daemon-model"],
            now,
            0,
            HEALTH_STATUS_HEALTHY,
            None,
        )
        .await;
        insert_daemon_node(
            &database,
            vec!["corrupt-daemon-model"],
            now,
            0,
            HEALTH_STATUS_HEALTHY,
            None,
        )
        .await;
        query(&prepare_sql(
            DatabaseBackend::Sqlite,
            "UPDATE daemon_nodes SET model_ids_json = ? WHERE model_ids_json = ?",
        ))
        .bind("{not-json")
        .bind("[\"corrupt-daemon-model\"]")
        .execute(&database)
        .await
        .expect("corrupt daemon model ids");

        let models = list_public_models(&database, DatabaseBackend::Sqlite, 60)
            .await
            .expect("list public models");
        let ids = models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["healthy-daemon-model"]);
    }
}
