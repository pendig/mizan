use axum::Json;
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use mizan_core::{AppError, ErrorEnvelope};
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{query, query_as};
use tracing::warn;
use uuid::Uuid;

use crate::AppState;
use crate::auth::ApiKeyIdentity;
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
const AUTH_MODE_SUBSCRIPTION_CLI: &str = "subscription_cli";
const AUTH_MODE_BROWSER_SESSION: &str = "browser_session";
const DAEMON_STATUS_ACTIVE: &str = "active";
const DAEMON_HEALTHY_STATUS: &str = "healthy";
const DAEMON_OWNED_BY: &str = "mizan-daemon";
const DAEMON_ROUTE_ID: &str = "daemon";

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
        AUTH_MODE_SUBSCRIPTION_CLI => Ok(AUTH_MODE_SUBSCRIPTION_CLI),
        AUTH_MODE_BROWSER_SESSION => Ok(AUTH_MODE_BROWSER_SESSION),
        _ => Err(AppError::invalid_config(
            "provider_connection.auth_mode",
            "auth_mode must be api_key, subscription_cli, or browser_session",
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
        return Err(AppError::invalid_config(
            "provider_connection.auth_config_json",
            "auth_config_json is required for non-api provider auth modes",
        ));
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
    let rows =
        query_as::<_, (String, String, String, String, String, Option<i64>, String)>(&prepare_sql(
            state.database_backend(),
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
        .fetch_all(&state.database)
        .await
        .map_err(|error| from_app_error(AppError::infrastructure(error.to_string())))?;

    let mut data = Vec::with_capacity(rows.len());

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
        let created = parse_timestamp(&created_at).map_err(from_app_error)?;

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

    append_daemon_public_models(&state, &mut data).await?;
    data.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(Json(PublicModelsResponse {
        object: "list",
        data,
    }))
}

async fn append_daemon_public_models(
    state: &AppState,
    data: &mut Vec<PublicModelResponse>,
) -> ProviderHttpResult<()> {
    let mut seen_models = data
        .iter()
        .map(|model| model.id.clone())
        .collect::<HashSet<_>>();
    let cutoff = now_utc_epoch_seconds()
        .saturating_sub(i64::from(state.config.daemon_stale_seconds.max(1)))
        .to_string();

    let rows = query_as::<_, (String, String, String)>(&prepare_sql(
        state.database_backend(),
        "SELECT provider_family, model_ids_json, last_seen_at
         FROM daemon_nodes
         WHERE status = ?
           AND revoked = 0
           AND disabled = 0
           AND health_status = ?
           AND provider_family IS NOT NULL
           AND model_ids_json != ?
           AND max_concurrency IS NOT NULL
           AND max_concurrency > 0
           AND last_seen_at IS NOT NULL
           AND last_seen_at >= ?
         ORDER BY last_seen_at DESC, created_at ASC",
    ))
    .bind(DAEMON_STATUS_ACTIVE)
    .bind(DAEMON_HEALTHY_STATUS)
    .bind("[]")
    .bind(cutoff)
    .fetch_all(&state.database)
    .await
    .map_err(|error| from_app_error(AppError::infrastructure(error.to_string())))?;

    for (provider_family, model_ids_json, last_seen_at) in rows {
        let created = parse_timestamp(&last_seen_at).map_err(from_app_error)?;
        let model_ids = parse_daemon_model_ids(&model_ids_json).map_err(from_app_error)?;

        for model_id in model_ids {
            if !seen_models.insert(model_id.clone()) {
                continue;
            }

            data.push(PublicModelResponse {
                id: model_id.clone(),
                object: "model",
                created,
                owned_by: DAEMON_OWNED_BY.to_owned(),
                provider_type: provider_family.clone(),
                upstream_model: model_id,
                route_id: DAEMON_ROUTE_ID.to_owned(),
                max_tokens: None,
            });
        }
    }

    Ok(())
}

fn parse_daemon_model_ids(raw: &str) -> Result<Vec<String>, AppError> {
    serde_json::from_str::<Vec<String>>(raw).map_err(|error| {
        AppError::infrastructure(format!("daemon_node.model_ids_json is invalid: {error}"))
    })
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
    let provider_secret_key = state.config.provider_secret_key.as_deref().ok_or_else(|| {
        from_app_error(AppError::invalid_config(
            "MIZAN_PROVIDER_SECRET_KEY",
            "set MIZAN_PROVIDER_SECRET_KEY before creating provider connections",
        ))
    })?;
    let secret_material = if auth_mode == AUTH_MODE_API_KEY {
        secret
    } else {
        ""
    };
    let encrypted_api_key =
        encrypt_provider_api_key(provider_secret_key, &id.to_string(), secret_material)
            .map_err(from_app_error)?;

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
    use crate::{metrics::MetricsRegistry, storage};
    use mizan_core::{AppConfig, DatabaseBackend};
    use mizan_gateway::Gateway;
    use redis::Client as RedisClient;

    async fn test_state() -> AppState {
        let database = storage::connect_and_migrate("sqlite::memory:", true, 1)
            .await
            .expect("create sqlite test database");
        let redis = RedisClient::open("redis://127.0.0.1:6379/")
            .expect("create redis client for state");

        AppState {
            config: AppConfig {
                http_addr: "127.0.0.1:0".parse().expect("parse test addr"),
                database_backend: DatabaseBackend::Sqlite,
                database_url: "sqlite::memory:".to_owned(),
                database_max_connections: 1,
                run_migrations: true,
                redis_url: "redis://127.0.0.1:6379/".to_owned(),
                limit_rpm: 0,
                limit_tpm: 0,
                limit_concurrency: 0,
                limit_window_seconds: 60,
                limit_lease_seconds: 120,
                log_level: "off".to_owned(),
                admin_seed_email: None,
                admin_seed_password: None,
                admin_seed_role: "admin".to_owned(),
                provider_secret_key: Some("test-provider-secret".to_owned()),
                log_raw_request_bodies: false,
                daemon_stale_seconds: 90,
            },
            gateway: Gateway::new(),
            database,
            redis,
            metrics: MetricsRegistry::default(),
        }
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
    async fn list_models_includes_only_safe_fresh_daemon_models() {
        let state = test_state().await;
        let now = unix_timestamp_string();
        let stale = (now_utc_epoch_seconds() - 300).to_string();
        let provider_id = Uuid::now_v7();
        let route_id = Uuid::now_v7();

        query(&prepare_sql(
            DatabaseBackend::Sqlite,
            "INSERT INTO provider_connections (
                 id, name, provider_type, auth_mode, base_url, api_key_encrypted, enabled, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        ))
        .bind(provider_id.to_string())
        .bind("route-provider")
        .bind("openai-compatible")
        .bind(AUTH_MODE_API_KEY)
        .bind("http://127.0.0.1:18182")
        .bind("encrypted")
        .bind(1)
        .bind(&now)
        .bind(&now)
        .execute(&state.database)
        .await
        .expect("insert provider connection");

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
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        ))
        .bind(route_id.to_string())
        .bind(provider_id.to_string())
        .bind("routed-model")
        .bind("upstream-routed-model")
        .bind(4096_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(1)
        .bind(&now)
        .bind(&now)
        .execute(&state.database)
        .await
        .expect("insert model route");

        insert_daemon_node(
            &state,
            "fresh",
            &now,
            0,
            DAEMON_HEALTHY_STATUS,
            r#"["llama3.1","qwen2.5-coder"]"#,
        )
        .await;
        insert_daemon_node(
            &state,
            "stale",
            &stale,
            0,
            DAEMON_HEALTHY_STATUS,
            r#"["stale-model"]"#,
        )
        .await;
        insert_daemon_node(
            &state,
            "disabled",
            &now,
            1,
            DAEMON_HEALTHY_STATUS,
            r#"["disabled-model"]"#,
        )
        .await;
        insert_daemon_node(
            &state,
            "unhealthy",
            &now,
            0,
            "degraded",
            r#"["unhealthy-model"]"#,
        )
        .await;

        let response = list_models(axum::extract::State(state))
            .await
            .expect("list public models")
            .0;

        let ids = response
            .data
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["llama3.1", "qwen2.5-coder", "routed-model"]);

        let daemon_model = response
            .data
            .iter()
            .find(|model| model.id == "llama3.1")
            .expect("daemon model included");
        assert_eq!(daemon_model.owned_by, DAEMON_OWNED_BY);
        assert_eq!(daemon_model.provider_type, "openai-compatible");
        assert_eq!(daemon_model.upstream_model, "llama3.1");
        assert_eq!(daemon_model.route_id, DAEMON_ROUTE_ID);
        assert_eq!(daemon_model.max_tokens, None);
    }

    async fn insert_daemon_node(
        state: &AppState,
        label: &str,
        last_seen_at: &str,
        disabled: i64,
        health_status: &str,
        model_ids_json: &str,
    ) {
        let now = unix_timestamp_string();
        query(&prepare_sql(
            DatabaseBackend::Sqlite,
            "INSERT INTO daemon_nodes (
                 id,
                 label,
                 token_hash,
                 status,
                 revoked,
                 last_seen_at,
                 created_at,
                 updated_at,
                 provider_family,
                 model_ids_json,
                 max_concurrency,
                 health_status,
                 disabled,
                 hostname,
                 labels_json,
                 capability_metadata_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        ))
        .bind(Uuid::now_v7().to_string())
        .bind(label)
        .bind(format!("hash-{label}"))
        .bind(DAEMON_STATUS_ACTIVE)
        .bind(0_i64)
        .bind(last_seen_at)
        .bind(&now)
        .bind(&now)
        .bind("openai-compatible")
        .bind(model_ids_json)
        .bind(2_i64)
        .bind(health_status)
        .bind(disabled)
        .bind(format!("{label}.internal"))
        .bind(r#"["private-label"]"#)
        .bind(r#"{"local_provider_url":"http://127.0.0.1:11434/v1"}"#)
        .execute(&state.database)
        .await
        .expect("insert daemon node");
    }
}
