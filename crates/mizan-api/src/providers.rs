use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use mizan_core::{AppError, AppResult, DatabaseBackend, ErrorEnvelope};
use serde::{Deserialize, Serialize};
use sqlx::{query, query_as};
use uuid::Uuid;

use crate::AppState;
use crate::auth::ApiKeyIdentity;

type ProviderHttpResult<T> = Result<T, (StatusCode, Json<ErrorEnvelope>)>;

#[derive(Debug, Serialize)]
pub struct ProviderConnectionResponse {
    pub id: String,
    pub name: String,
    pub provider_type: String,
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
    pub base_url: String,
    pub api_key_encrypted: String,
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
    axum::Extension(identity): axum::Extension<ApiKeyIdentity>,
    request: axum::http::Request<Body>,
    next: Next,
) -> ProviderHttpResult<Response> {
    if identity.user_role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorEnvelope::from(&AppError::Forbidden)),
        ));
    }

    Ok(next.run(request).await)
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
             WHERE mr.enabled = 1 AND pc.enabled = 1
             ORDER BY mr.public_model ASC",
        ))
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
        let created = parse_timestamp(&created_at).map_err(|error| from_app_error(error))?;

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

    Ok(Json(PublicModelsResponse {
        object: "list",
        data,
    }))
}

pub async fn list_provider_connections(
    State(state): State<AppState>,
) -> ProviderHttpResult<Json<ProviderConnectionListResponse>> {
    let rows = query_as::<_, (String, String, String, String, i64, String, String)>(&prepare_sql(
        state.database_backend(),
        "SELECT id,
                    name,
                    provider_type,
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
            |(id, name, provider_type, base_url, enabled, created_at, updated_at)| {
                ProviderConnectionResponse {
                    id,
                    name,
                    provider_type,
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
    Json(payload): Json<ProviderConnectionCreateRequest>,
) -> ProviderHttpResult<Json<ProviderConnectionCreateResponse>> {
    let name = payload.name.trim();
    let provider_type = payload.provider_type.trim();
    let base_url = payload.base_url.trim();
    let secret = payload.api_key_encrypted.trim();

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

    if base_url.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "provider_connection.base_url",
                "base_url is required",
            ))),
        ));
    }

    if secret.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "provider_connection.api_key_encrypted",
                "api_key_encrypted is required",
            ))),
        ));
    }

    let id = Uuid::now_v7();
    let now = unix_timestamp_string();
    let enabled = payload.enabled.unwrap_or(true);

    let sql = prepare_sql(
        state.database_backend(),
        "INSERT INTO provider_connections (
             id, name, provider_type, base_url, api_key_encrypted, enabled, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    );

    query(&sql)
        .bind(id.to_string())
        .bind(name)
        .bind(provider_type)
        .bind(base_url)
        .bind(secret)
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

    Ok(Json(ProviderConnectionCreateResponse {
        id: id.to_string(),
        name: name.to_string(),
        provider_type: provider_type.to_string(),
        base_url: base_url.to_string(),
        enabled,
    }))
}

pub async fn delete_provider_connection(
    State(state): State<AppState>,
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

    if let Some(max_tokens) = payload.max_tokens {
        if max_tokens < 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorEnvelope::from(&AppError::invalid_config(
                    "model_route.max_tokens",
                    "max_tokens cannot be negative",
                ))),
            ));
        }
    }

    let provider_connection_id = payload.provider_connection_id;

    let provider_exists = query_as::<_, (i64,)>(&prepare_sql(
        state.database_backend(),
        "SELECT 1 FROM provider_connections WHERE id = ?",
    ))
    .bind(provider_connection_id.to_string())
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

fn from_app_error(error: AppError) -> (StatusCode, Json<ErrorEnvelope>) {
    let status = match error {
        AppError::InvalidConfig { .. } => StatusCode::BAD_REQUEST,
        AppError::NotFound(_) => StatusCode::NOT_FOUND,
        AppError::Unauthorized => StatusCode::UNAUTHORIZED,
        AppError::Forbidden => StatusCode::FORBIDDEN,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };

    (status, Json(ErrorEnvelope::from(&error)))
}

fn is_enabled(raw: i64) -> bool {
    raw != 0
}

fn parse_timestamp(raw: &str) -> AppResult<i64> {
    raw.parse::<i64>()
        .map_err(|error| AppError::infrastructure(format!("invalid timestamp: {error}")))
}

fn prepare_sql(database_backend: DatabaseBackend, query: &'static str) -> String {
    match database_backend {
        DatabaseBackend::Sqlite => query.to_string(),
        DatabaseBackend::Postgres => to_dollar_params(query),
    }
}

fn to_dollar_params(query: &str) -> String {
    let mut parameter_index = 0usize;
    let mut converted = String::with_capacity(query.len());

    for character in query.chars() {
        if character == '?' {
            parameter_index += 1;
            converted.push('$');
            converted.push_str(&parameter_index.to_string());
            continue;
        }

        converted.push(character);
    }

    converted
}

fn is_unique_constraint_error(message: &str) -> bool {
    let normalized = message.to_lowercase();
    normalized.contains("unique")
        && (normalized.contains("constraint") || normalized.contains("already exists"))
}

fn unix_timestamp_string() -> String {
    now_utc_epoch_seconds().to_string()
}

fn now_utc_epoch_seconds() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_sql_keeps_question_marks_for_sqlite() {
        let prepared = prepare_sql(DatabaseBackend::Sqlite, "SELECT * FROM x WHERE id = ?");
        assert_eq!(prepared, "SELECT * FROM x WHERE id = ?");
    }

    #[test]
    fn prepare_sql_converts_question_marks_for_postgres() {
        let prepared = prepare_sql(
            DatabaseBackend::Postgres,
            "SELECT * FROM x WHERE a = ? AND b = ?",
        );
        assert_eq!(prepared, "SELECT * FROM x WHERE a = $1 AND b = $2");
    }

    #[test]
    fn unix_timestamp_string_is_numeric() {
        let timestamp = unix_timestamp_string();
        assert!(timestamp.parse::<i64>().is_ok());
    }
}
