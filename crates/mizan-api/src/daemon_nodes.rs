use axum::Json;
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::http::{Request, StatusCode, header::AUTHORIZATION};
use axum::middleware::Next;
use axum::response::Response;
use mizan_core::{AppError, DatabaseBackend, ErrorEnvelope};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{AnyPool, FromRow, query, query_as};
use tracing::{Instrument, info_span, warn};
use uuid::Uuid;

use crate::AppState;
use crate::auth::ApiKeyIdentity;
use crate::logging::{AdminAuditInput, record_admin_audit, serialize_payload};
use crate::utils::{
    from_app_error, is_enabled, is_unique_constraint_error, now_utc_epoch_seconds, prepare_sql,
    unix_timestamp_string,
};

type DaemonNodeHttpResult<T> = Result<T, (StatusCode, Json<ErrorEnvelope>)>;

const DAEMON_TOKEN_PREFIX: &str = "mizan_sk_daemon_";
const STATUS_PENDING: &str = "pending";
const STATUS_ACTIVE: &str = "active";
const STATUS_REVOKED: &str = "revoked";
const HEALTH_STATUS_HEALTHY: &str = "healthy";
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
    pub disabled: bool,
    pub last_seen_at: Option<String>,
    pub capabilities: DaemonCapabilityResponse,
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
    #[serde(default)]
    pub capabilities: Option<DaemonCapabilityPayload>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonRegistrationResponse {
    pub node_id: Uuid,
    pub status: String,
    pub last_seen_at: String,
    pub capabilities: DaemonCapabilityResponse,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonPingResponse {
    pub node_id: Uuid,
    pub status: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonHeartbeatRequest {
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub public_key: Option<String>,
    pub capabilities: DaemonCapabilityPayload,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonHeartbeatResponse {
    pub node_id: Uuid,
    pub status: String,
    pub last_seen_at: String,
    pub capabilities: DaemonCapabilityResponse,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonCapabilityPayload {
    pub provider_family: String,
    pub model_ids: Vec<String>,
    pub max_concurrency: u32,
    #[serde(default)]
    pub pricing_metadata: Option<Value>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub health_status: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DaemonCapabilityResponse {
    pub provider_family: Option<String>,
    pub model_ids: Vec<String>,
    pub max_concurrency: Option<u32>,
    pub pricing_metadata: Option<Value>,
    pub region: Option<String>,
    pub labels: Vec<String>,
    pub health_status: Option<String>,
    pub metadata: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibleDaemonNode {
    pub id: Uuid,
    pub provider_family: String,
    pub model_id: String,
    pub max_concurrency: u32,
    pub last_seen_at: String,
}

#[derive(Debug, Clone)]
pub struct DaemonNodeIdentity {
    pub node_id: Uuid,
    pub status: String,
}

#[derive(Debug, FromRow)]
struct DbDaemonNode {
    id: String,
    host_user_id: Option<String>,
    label: Option<String>,
    hostname: Option<String>,
    public_key: Option<String>,
    status: String,
    revoked: i32,
    disabled: i32,
    last_seen_at: Option<String>,
    provider_family: Option<String>,
    model_ids_json: String,
    max_concurrency: Option<i32>,
    pricing_metadata_json: Option<String>,
    region: Option<String>,
    labels_json: String,
    health_status: Option<String>,
    capability_metadata_json: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone)]
struct NormalizedCapabilities {
    provider_family: String,
    model_ids: Vec<String>,
    max_concurrency: u32,
    pricing_metadata: Option<Value>,
    region: Option<String>,
    labels: Vec<String>,
    health_status: String,
    metadata: Option<Value>,
}

pub async fn list_daemon_nodes(
    State(state): State<AppState>,
) -> DaemonNodeHttpResult<Json<DaemonNodeListResponse>> {
    let rows = query_as::<_, DbDaemonNode>(&prepare_sql(
        state.database_backend(),
        "SELECT id,
                host_user_id,
                label,
                hostname,
                public_key,
                status,
                revoked,
                disabled,
                last_seen_at,
                provider_family,
                model_ids_json,
                max_concurrency,
                pricing_metadata_json,
                region,
                labels_json,
                health_status,
                capability_metadata_json,
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
        .map(daemon_node_response)
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
    let capabilities = payload
        .capabilities
        .map(normalize_capabilities)
        .transpose()
        .map_err(from_app_error)?;
    let last_seen_at = mark_node_seen(
        &state.database,
        state.database_backend(),
        identity.node_id,
        normalize_optional(payload.hostname),
        normalize_optional(payload.public_key),
        capabilities.as_ref(),
    )
    .await
    .map_err(from_app_error)?;

    Ok(Json(DaemonRegistrationResponse {
        node_id: identity.node_id,
        status: STATUS_ACTIVE.to_owned(),
        last_seen_at,
        capabilities: capabilities
            .map(normalized_capability_response)
            .unwrap_or_default(),
    }))
}

pub async fn daemon_heartbeat(
    State(state): State<AppState>,
    Extension(identity): Extension<DaemonNodeIdentity>,
    Json(payload): Json<DaemonHeartbeatRequest>,
) -> DaemonNodeHttpResult<Json<DaemonHeartbeatResponse>> {
    let capabilities = normalize_capabilities(payload.capabilities).map_err(from_app_error)?;
    let last_seen_at = mark_node_seen(
        &state.database,
        state.database_backend(),
        identity.node_id,
        normalize_optional(payload.hostname),
        normalize_optional(payload.public_key),
        Some(&capabilities),
    )
    .await
    .map_err(from_app_error)?;

    Ok(Json(DaemonHeartbeatResponse {
        node_id: identity.node_id,
        status: STATUS_ACTIVE.to_owned(),
        last_seen_at,
        capabilities: normalized_capability_response(capabilities),
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
    capabilities: Option<&NormalizedCapabilities>,
) -> Result<String, AppError> {
    let now = unix_timestamp_string();

    let result = if let Some(capabilities) = capabilities {
        query(&prepare_sql(
            database_backend,
            "UPDATE daemon_nodes
             SET status = ?,
                 last_seen_at = ?,
                 hostname = COALESCE(?, hostname),
                 public_key = COALESCE(?, public_key),
                 provider_family = ?,
                 model_ids_json = ?,
                 max_concurrency = ?,
                 pricing_metadata_json = ?,
                 region = ?,
                 labels_json = ?,
                 health_status = ?,
                 capability_metadata_json = ?,
                 updated_at = ?
             WHERE id = ? AND revoked = 0 AND status != ?",
        ))
        .bind(STATUS_ACTIVE)
        .bind(&now)
        .bind(hostname.as_deref())
        .bind(public_key.as_deref())
        .bind(&capabilities.provider_family)
        .bind(serialize_json(&capabilities.model_ids)?)
        .bind(i32::try_from(capabilities.max_concurrency).map_err(|_| {
            AppError::invalid_config(
                "daemon_capabilities.max_concurrency",
                "max_concurrency exceeds database integer range",
            )
        })?)
        .bind(serialize_optional_json(
            capabilities.pricing_metadata.as_ref(),
        )?)
        .bind(capabilities.region.as_deref())
        .bind(serialize_json(&capabilities.labels)?)
        .bind(&capabilities.health_status)
        .bind(serialize_optional_json(capabilities.metadata.as_ref())?)
        .bind(&now)
        .bind(node_id.to_string())
        .bind(STATUS_REVOKED)
        .execute(database)
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?
    } else {
        query(&prepare_sql(
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
        .map_err(|error| AppError::infrastructure(error.to_string()))?
    };

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
    let capabilities = daemon_capability_response(&row)?;
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
        disabled: is_enabled(row.disabled),
        last_seen_at: row.last_seen_at,
        capabilities,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[allow(dead_code)]
pub async fn select_eligible_daemon_node(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    model_id: &str,
    stale_after_seconds: i64,
) -> Result<Option<EligibleDaemonNode>, AppError> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Ok(None);
    }

    let cutoff = now_utc_epoch_seconds().saturating_sub(stale_after_seconds.max(1));
    let cutoff = cutoff.to_string();
    let rows = query_as::<_, (String, String, String, i32, String)>(&prepare_sql(
        database_backend,
        "SELECT id, provider_family, model_ids_json, max_concurrency, last_seen_at
         FROM daemon_nodes
         WHERE status = ?
           AND revoked = 0
           AND disabled = 0
           AND health_status = ?
           AND provider_family IS NOT NULL
           AND max_concurrency IS NOT NULL
           AND last_seen_at IS NOT NULL
           AND last_seen_at >= ?
         ORDER BY last_seen_at DESC, created_at ASC",
    ))
    .bind(STATUS_ACTIVE)
    .bind(HEALTH_STATUS_HEALTHY)
    .bind(&cutoff)
    .fetch_all(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    for (id, provider_family, model_ids_json, max_concurrency, last_seen_at) in rows {
        let model_ids = parse_json_vec(&model_ids_json, "daemon_node.model_ids_json")?;
        if !model_ids.iter().any(|candidate| candidate == model_id) {
            continue;
        }

        let max_concurrency = u32::try_from(max_concurrency)
            .map_err(|_| AppError::infrastructure("stored daemon max_concurrency is invalid"))?;
        if max_concurrency == 0 {
            continue;
        }

        let id = Uuid::parse_str(&id).map_err(|error| {
            AppError::infrastructure(format!("stored daemon node id is invalid: {error}"))
        })?;

        return Ok(Some(EligibleDaemonNode {
            id,
            provider_family,
            model_id: model_id.to_owned(),
            max_concurrency,
            last_seen_at,
        }));
    }

    Ok(None)
}

fn normalized_capability_response(value: NormalizedCapabilities) -> DaemonCapabilityResponse {
    DaemonCapabilityResponse {
        provider_family: Some(value.provider_family),
        model_ids: value.model_ids,
        max_concurrency: Some(value.max_concurrency),
        pricing_metadata: value.pricing_metadata,
        region: value.region,
        labels: value.labels,
        health_status: Some(value.health_status),
        metadata: value.metadata,
    }
}

fn daemon_capability_response(row: &DbDaemonNode) -> Result<DaemonCapabilityResponse, AppError> {
    let max_concurrency = row
        .max_concurrency
        .map(|value| {
            u32::try_from(value)
                .map_err(|_| AppError::infrastructure("stored daemon max_concurrency is invalid"))
        })
        .transpose()?;

    Ok(DaemonCapabilityResponse {
        provider_family: row.provider_family.clone(),
        model_ids: parse_json_vec(&row.model_ids_json, "daemon_node.model_ids_json")?,
        max_concurrency,
        pricing_metadata: parse_optional_json_value(
            row.pricing_metadata_json.as_deref(),
            "daemon_node.pricing_metadata_json",
        )?,
        region: row.region.clone(),
        labels: parse_json_vec(&row.labels_json, "daemon_node.labels_json")?,
        health_status: row.health_status.clone(),
        metadata: parse_optional_json_value(
            row.capability_metadata_json.as_deref(),
            "daemon_node.capability_metadata_json",
        )?,
    })
}

fn normalize_capabilities(
    payload: DaemonCapabilityPayload,
) -> Result<NormalizedCapabilities, AppError> {
    let provider_family = payload.provider_family.trim().to_ascii_lowercase();
    if provider_family.is_empty() {
        return Err(AppError::invalid_config(
            "daemon_capabilities.provider_family",
            "provider_family is required",
        ));
    }

    let model_ids = normalize_string_list(payload.model_ids);
    if model_ids.is_empty() {
        return Err(AppError::invalid_config(
            "daemon_capabilities.model_ids",
            "at least one model id is required",
        ));
    }

    if payload.max_concurrency == 0 {
        return Err(AppError::invalid_config(
            "daemon_capabilities.max_concurrency",
            "must be greater than zero",
        ));
    }

    let health_status = normalize_optional(payload.health_status)
        .unwrap_or_else(|| HEALTH_STATUS_HEALTHY.to_owned())
        .to_ascii_lowercase();

    Ok(NormalizedCapabilities {
        provider_family,
        model_ids,
        max_concurrency: payload.max_concurrency,
        pricing_metadata: payload.pricing_metadata,
        region: normalize_optional(payload.region),
        labels: normalize_string_list(payload.labels),
        health_status,
        metadata: payload.metadata,
    })
}

fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty()
            || normalized
                .iter()
                .any(|candidate: &String| candidate.as_str() == value)
        {
            continue;
        }
        normalized.push(value.to_owned());
    }
    normalized
}

fn serialize_json(value: &impl Serialize) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|error| AppError::infrastructure(format!("json serialization failed: {error}")))
}

fn serialize_optional_json(value: Option<&Value>) -> Result<Option<String>, AppError> {
    value.map(serialize_json).transpose()
}

fn parse_json_vec(raw: &str, field_name: &'static str) -> Result<Vec<String>, AppError> {
    serde_json::from_str(raw)
        .map_err(|error| AppError::infrastructure(format!("{field_name} is invalid: {error}")))
}

fn parse_optional_json_value(
    raw: Option<&str>,
    field_name: &'static str,
) -> Result<Option<Value>, AppError> {
    raw.map(|value| {
        serde_json::from_str(value)
            .map_err(|error| AppError::infrastructure(format!("{field_name} is invalid: {error}")))
    })
    .transpose()
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
            None,
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
    async fn daemon_heartbeat_stores_capabilities() {
        let database = sqlite_test_database().await;
        let token = "mizan_sk_daemon_capable";
        let node_id = insert_node(&database, token, false).await;
        let capabilities = normalize_capabilities(DaemonCapabilityPayload {
            provider_family: "openai-compatible".to_owned(),
            model_ids: vec![" llama3.1 ".to_owned(), "qwen2.5-coder".to_owned()],
            max_concurrency: 4,
            pricing_metadata: Some(json!({"input_per_1m": 100})),
            region: Some("iad".to_owned()),
            labels: vec!["gpu".to_owned()],
            health_status: None,
            metadata: Some(json!({"local_provider_url": "http://127.0.0.1:11434/v1"})),
        })
        .expect("valid capabilities");

        mark_node_seen(
            &database,
            DatabaseBackend::Sqlite,
            node_id,
            Some("host-b".to_owned()),
            None,
            Some(&capabilities),
        )
        .await
        .expect("mark seen with capabilities");

        let row: (String, String, i64, String, String) = query_as(
            "SELECT provider_family, model_ids_json, max_concurrency, health_status, region
             FROM daemon_nodes WHERE id = ?",
        )
        .bind(node_id.to_string())
        .fetch_one(&database)
        .await
        .expect("read daemon capabilities");

        assert_eq!(row.0, "openai-compatible");
        assert_eq!(row.1, r#"["llama3.1","qwen2.5-coder"]"#);
        assert_eq!(row.2, 4);
        assert_eq!(row.3, HEALTH_STATUS_HEALTHY);
        assert_eq!(row.4, "iad");
    }

    #[tokio::test]
    async fn daemon_capability_validation_rejects_empty_models_and_zero_capacity() {
        let empty_models = normalize_capabilities(DaemonCapabilityPayload {
            provider_family: "openai-compatible".to_owned(),
            model_ids: vec![" ".to_owned()],
            max_concurrency: 1,
            pricing_metadata: None,
            region: None,
            labels: Vec::new(),
            health_status: None,
            metadata: None,
        });
        assert!(empty_models.is_err());

        let zero_capacity = normalize_capabilities(DaemonCapabilityPayload {
            provider_family: "openai-compatible".to_owned(),
            model_ids: vec!["llama3.1".to_owned()],
            max_concurrency: 0,
            pricing_metadata: None,
            region: None,
            labels: Vec::new(),
            health_status: None,
            metadata: None,
        });
        assert!(zero_capacity.is_err());
    }

    #[tokio::test]
    async fn daemon_selection_excludes_stale_disabled_and_unhealthy_nodes() {
        let database = sqlite_test_database().await;
        let now = now_utc_epoch_seconds();
        let online =
            insert_selectable_node(&database, "llama3.1", now, 0, HEALTH_STATUS_HEALTHY).await;
        insert_selectable_node(&database, "llama3.1", now - 120, 0, HEALTH_STATUS_HEALTHY).await;
        insert_selectable_node(&database, "llama3.1", now, 1, HEALTH_STATUS_HEALTHY).await;
        insert_selectable_node(&database, "llama3.1", now, 0, "degraded").await;

        let selected =
            select_eligible_daemon_node(&database, DatabaseBackend::Sqlite, "llama3.1", 60)
                .await
                .expect("select daemon node")
                .expect("online node should be selected");

        assert_eq!(selected.id, online);
    }

    #[tokio::test]
    async fn daemon_selection_returns_none_when_only_stale_nodes_match() {
        let database = sqlite_test_database().await;
        insert_selectable_node(
            &database,
            "self-hosted/gpt-oss",
            now_utc_epoch_seconds() - 120,
            0,
            HEALTH_STATUS_HEALTHY,
        )
        .await;

        let selected = select_eligible_daemon_node(
            &database,
            DatabaseBackend::Sqlite,
            "self-hosted/gpt-oss",
            60,
        )
        .await
        .expect("select daemon node");

        assert!(selected.is_none());
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

    async fn insert_selectable_node(
        database: &AnyPool,
        model_id: &str,
        last_seen_at: i64,
        disabled: i32,
        health_status: &str,
    ) -> Uuid {
        let node_id = Uuid::now_v7();
        let user_id = seed_user(database).await;
        let now = unix_timestamp_string();
        query(&prepare_sql(
            DatabaseBackend::Sqlite,
            "INSERT INTO daemon_nodes (
                 id,
                 host_user_id,
                 token_hash,
                 status,
                 revoked,
                 disabled,
                 last_seen_at,
                 provider_family,
                 model_ids_json,
                 max_concurrency,
                 health_status,
                 created_at,
                 updated_at
             ) VALUES (?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?, ?)",
        ))
        .bind(node_id.to_string())
        .bind(user_id.to_string())
        .bind(hash_value(&format!("token-{node_id}")))
        .bind(STATUS_ACTIVE)
        .bind(disabled)
        .bind(last_seen_at.to_string())
        .bind("openai-compatible")
        .bind(serialize_json(&vec![model_id.to_owned()]).expect("serialize model ids"))
        .bind(4_i32)
        .bind(health_status)
        .bind(&now)
        .bind(&now)
        .execute(database)
        .await
        .expect("insert selectable daemon node");
        node_id
    }
}
