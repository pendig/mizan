use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use mizan_core::{AppError, DatabaseBackend, ErrorEnvelope};
use mizan_providers::{ChatRequest, ChatResponse};
use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, FromRow, query, query_as};
use tokio::time::{Duration, Instant, sleep};
use tracing::{Instrument, info_span};
use uuid::Uuid;

use crate::AppState;
use crate::daemon_nodes::DaemonNodeIdentity;
use crate::utils::{from_app_error, now_utc_epoch_seconds, prepare_sql, unix_timestamp_string};

const STATUS_ACCEPTED: &str = "accepted";
const STATUS_LEASED: &str = "leased";
const STATUS_RUNNING: &str = "running";
const STATUS_SUCCEEDED: &str = "succeeded";
const STATUS_FAILED: &str = "failed";
const STATUS_TIMED_OUT: &str = "timed_out";
const DISPATCH_POLL_INTERVAL: Duration = Duration::from_millis(50);

type DispatchHttpResult<T> = Result<T, (StatusCode, Json<ErrorEnvelope>)>;

#[derive(Debug, Clone, Serialize)]
pub struct DispatchRecord {
    pub id: Uuid,
    pub request_id: Uuid,
    pub node_id: Uuid,
    pub model: String,
    pub status: String,
    pub latency_ms: Option<u64>,
    pub error_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DispatchJobResponse {
    pub id: Uuid,
    pub request_id: Uuid,
    pub model: String,
    pub request: ChatRequest,
    pub timeout_at: String,
}

#[derive(Debug, Serialize)]
pub struct NoDispatchJobResponse {
    pub job: Option<DispatchJobResponse>,
}

#[derive(Debug, Deserialize)]
pub struct DispatchCompleteRequest {
    pub response: ChatResponse,
}

#[derive(Debug, Deserialize)]
pub struct DispatchFailRequest {
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, FromRow)]
struct DispatchJobRow {
    id: String,
    request_id: String,
    node_id: String,
    model: String,
    status: String,
    request_json: String,
    response_json: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
    timeout_at: String,
    created_at: String,
}

pub async fn daemon_next_job(
    State(state): State<AppState>,
    Extension(identity): Extension<DaemonNodeIdentity>,
) -> DispatchHttpResult<Json<NoDispatchJobResponse>> {
    expire_timed_out_jobs(&state.database, state.database_backend())
        .await
        .map_err(from_app_error)?;

    let now = unix_timestamp_string();
    let row = query_as::<_, DispatchJobRow>(&prepare_sql(
        state.database_backend(),
        "SELECT id,
                request_id,
                node_id,
                model,
                status,
                request_json,
                response_json,
                error_code,
                error_message,
                timeout_at,
                created_at
         FROM dispatch_jobs
         WHERE node_id = ?
           AND status = ?
           AND timeout_at > ?
         ORDER BY created_at ASC
         LIMIT 1",
    ))
    .bind(identity.node_id.to_string())
    .bind(STATUS_LEASED)
    .bind(&now)
    .fetch_optional(&state.database)
    .await
    .map_err(|error| from_app_error(AppError::infrastructure(error.to_string())))?;

    let Some(row) = row else {
        return Ok(Json(NoDispatchJobResponse { job: None }));
    };

    query(&prepare_sql(
        state.database_backend(),
        "UPDATE dispatch_jobs
         SET status = ?, started_at = ?, updated_at = ?
         WHERE id = ? AND status = ?",
    ))
    .bind(STATUS_RUNNING)
    .bind(&now)
    .bind(&now)
    .bind(&row.id)
    .bind(STATUS_LEASED)
    .execute(&state.database)
    .await
    .map_err(|error| from_app_error(AppError::infrastructure(error.to_string())))?;

    Ok(Json(NoDispatchJobResponse {
        job: Some(dispatch_job_response(row).map_err(from_app_error)?),
    }))
}

pub async fn daemon_complete_job(
    State(state): State<AppState>,
    Extension(identity): Extension<DaemonNodeIdentity>,
    Path(id): Path<Uuid>,
    Json(payload): Json<DispatchCompleteRequest>,
) -> DispatchHttpResult<Json<DispatchRecord>> {
    let response_json = serde_json::to_string(&payload.response).map_err(|error| {
        from_app_error(AppError::invalid_config(
            "dispatch.response",
            format!("response is not serializable: {error}"),
        ))
    })?;
    let now = unix_timestamp_string();
    let latency_ms = dispatch_latency_ms(
        &state.database,
        state.database_backend(),
        id,
        now_utc_epoch_seconds(),
    )
    .await
    .map_err(from_app_error)?;

    let result = query(&prepare_sql(
        state.database_backend(),
        "UPDATE dispatch_jobs
         SET status = ?,
             response_json = ?,
             completed_at = ?,
             latency_ms = ?,
             updated_at = ?
         WHERE id = ?
           AND node_id = ?
           AND status IN (?, ?)",
    ))
    .bind(STATUS_SUCCEEDED)
    .bind(response_json)
    .bind(&now)
    .bind(i64::try_from(latency_ms).unwrap_or(i64::MAX))
    .bind(&now)
    .bind(id.to_string())
    .bind(identity.node_id.to_string())
    .bind(STATUS_LEASED)
    .bind(STATUS_RUNNING)
    .execute(&state.database)
    .await
    .map_err(|error| from_app_error(AppError::infrastructure(error.to_string())))?;

    if result.rows_affected() != 1 {
        return Err(from_app_error(AppError::NotFound(
            "dispatch job not found or already completed".to_owned(),
        )));
    }

    Ok(Json(DispatchRecord {
        id,
        request_id: Uuid::nil(),
        node_id: identity.node_id,
        model: String::new(),
        status: STATUS_SUCCEEDED.to_owned(),
        latency_ms: Some(latency_ms),
        error_code: None,
    }))
}

pub async fn daemon_fail_job(
    State(state): State<AppState>,
    Extension(identity): Extension<DaemonNodeIdentity>,
    Path(id): Path<Uuid>,
    Json(payload): Json<DispatchFailRequest>,
) -> DispatchHttpResult<Json<DispatchRecord>> {
    fail_dispatch_job(
        &state.database,
        state.database_backend(),
        id,
        identity.node_id,
        payload.error_code.as_deref().unwrap_or("daemon_failure"),
        payload
            .error_message
            .as_deref()
            .unwrap_or("daemon failed dispatch job"),
    )
    .await
    .map_err(from_app_error)?;

    Ok(Json(DispatchRecord {
        id,
        request_id: Uuid::nil(),
        node_id: identity.node_id,
        model: String::new(),
        status: STATUS_FAILED.to_owned(),
        latency_ms: None,
        error_code: payload.error_code,
    }))
}

pub async fn create_dispatch_job(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    request_id: Uuid,
    node_id: Uuid,
    model: &str,
    request: &ChatRequest,
    timeout_seconds: u64,
) -> Result<Uuid, AppError> {
    let id = Uuid::now_v7();
    let now = unix_timestamp_string();
    let timeout_at = now_utc_epoch_seconds()
        .saturating_add(i64::try_from(timeout_seconds.max(1)).unwrap_or(i64::MAX))
        .to_string();
    let request_json = serde_json::to_string(request).map_err(|error| {
        AppError::invalid_config(
            "dispatch.request",
            format!("request is not serializable: {error}"),
        )
    })?;

    query(&prepare_sql(
        database_backend,
        "INSERT INTO dispatch_jobs (
             id,
             request_id,
             node_id,
             model,
             status,
             request_json,
             leased_at,
             timeout_at,
             created_at,
             updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(id.to_string())
    .bind(request_id.to_string())
    .bind(node_id.to_string())
    .bind(model)
    .bind(STATUS_LEASED)
    .bind(request_json)
    .bind(&now)
    .bind(timeout_at)
    .bind(&now)
    .bind(&now)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    Ok(id)
}

pub async fn wait_for_dispatch_result(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    dispatch_id: Uuid,
    timeout_seconds: u64,
) -> Result<ChatResponse, AppError> {
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds.max(1));
    loop {
        expire_timed_out_jobs(database, database_backend).await?;
        let row = query_as::<_, DispatchJobRow>(&prepare_sql(
            database_backend,
            "SELECT id,
                    request_id,
                    node_id,
                    model,
                    status,
                    request_json,
                    response_json,
                    error_code,
                    error_message,
                    timeout_at,
                    created_at
             FROM dispatch_jobs
             WHERE id = ?",
        ))
        .bind(dispatch_id.to_string())
        .fetch_optional(database)
        .instrument(info_span!("dispatch_wait", dispatch_id = %dispatch_id))
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?
        .ok_or_else(|| AppError::NotFound("dispatch job not found".to_owned()))?;

        match row.status.as_str() {
            STATUS_SUCCEEDED => {
                let raw = row.response_json.ok_or_else(|| {
                    AppError::infrastructure("dispatch job succeeded without response")
                })?;
                return serde_json::from_str(&raw).map_err(|error| {
                    AppError::infrastructure(format!("invalid dispatch response: {error}"))
                });
            }
            STATUS_FAILED => {
                return Err(AppError::provider(row.error_message.unwrap_or_else(|| {
                    row.error_code.unwrap_or_else(|| "daemon_failure".to_owned())
                })));
            }
            STATUS_TIMED_OUT => {
                return Err(AppError::provider("daemon dispatch timed out"));
            }
            _ if Instant::now() >= deadline => {
                mark_dispatch_timed_out(database, database_backend, dispatch_id).await?;
                return Err(AppError::provider("daemon dispatch timed out"));
            }
            _ => sleep(DISPATCH_POLL_INTERVAL).await,
        }
    }
}

pub async fn active_dispatch_count_for_node(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    node_id: Uuid,
) -> Result<i64, AppError> {
    expire_timed_out_jobs(database, database_backend).await?;
    let row = query_as::<_, (i64,)>(&prepare_sql(
        database_backend,
        "SELECT COUNT(*)
         FROM dispatch_jobs
         WHERE node_id = ?
           AND status IN (?, ?, ?)",
    ))
    .bind(node_id.to_string())
    .bind(STATUS_ACCEPTED)
    .bind(STATUS_LEASED)
    .bind(STATUS_RUNNING)
    .fetch_one(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;
    Ok(row.0)
}

async fn fail_dispatch_job(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    dispatch_id: Uuid,
    node_id: Uuid,
    error_code: &str,
    error_message: &str,
) -> Result<(), AppError> {
    let now = unix_timestamp_string();
    let result = query(&prepare_sql(
        database_backend,
        "UPDATE dispatch_jobs
         SET status = ?,
             error_code = ?,
             error_message = ?,
             completed_at = ?,
             updated_at = ?
         WHERE id = ?
           AND node_id = ?
           AND status IN (?, ?)",
    ))
    .bind(STATUS_FAILED)
    .bind(error_code)
    .bind(error_message)
    .bind(&now)
    .bind(&now)
    .bind(dispatch_id.to_string())
    .bind(node_id.to_string())
    .bind(STATUS_LEASED)
    .bind(STATUS_RUNNING)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    if result.rows_affected() != 1 {
        return Err(AppError::NotFound(
            "dispatch job not found or already completed".to_owned(),
        ));
    }

    Ok(())
}

async fn dispatch_latency_ms(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    dispatch_id: Uuid,
    now_seconds: i64,
) -> Result<u64, AppError> {
    let row = query_as::<_, (String,)>(&prepare_sql(
        database_backend,
        "SELECT created_at FROM dispatch_jobs WHERE id = ?",
    ))
    .bind(dispatch_id.to_string())
    .fetch_one(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;
    let created = row
        .0
        .parse::<i64>()
        .map_err(|error| AppError::infrastructure(format!("invalid dispatch timestamp: {error}")))?;
    Ok(now_seconds.saturating_sub(created).saturating_mul(1000) as u64)
}

async fn expire_timed_out_jobs(
    database: &AnyPool,
    database_backend: DatabaseBackend,
) -> Result<(), AppError> {
    let now = unix_timestamp_string();
    query(&prepare_sql(
        database_backend,
        "UPDATE dispatch_jobs
         SET status = ?,
             error_code = ?,
             error_message = ?,
             completed_at = ?,
             updated_at = ?
         WHERE status IN (?, ?, ?)
           AND timeout_at <= ?",
    ))
    .bind(STATUS_TIMED_OUT)
    .bind("timeout")
    .bind("daemon dispatch timed out")
    .bind(&now)
    .bind(&now)
    .bind(STATUS_ACCEPTED)
    .bind(STATUS_LEASED)
    .bind(STATUS_RUNNING)
    .bind(&now)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;
    Ok(())
}

async fn mark_dispatch_timed_out(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    dispatch_id: Uuid,
) -> Result<(), AppError> {
    let now = unix_timestamp_string();
    query(&prepare_sql(
        database_backend,
        "UPDATE dispatch_jobs
         SET status = ?,
             error_code = ?,
             error_message = ?,
             completed_at = ?,
             updated_at = ?
         WHERE id = ?
           AND status IN (?, ?, ?)",
    ))
    .bind(STATUS_TIMED_OUT)
    .bind("timeout")
    .bind("daemon dispatch timed out")
    .bind(&now)
    .bind(&now)
    .bind(dispatch_id.to_string())
    .bind(STATUS_ACCEPTED)
    .bind(STATUS_LEASED)
    .bind(STATUS_RUNNING)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;
    Ok(())
}

fn dispatch_job_response(row: DispatchJobRow) -> Result<DispatchJobResponse, AppError> {
    let id = Uuid::parse_str(&row.id)
        .map_err(|error| AppError::infrastructure(format!("invalid dispatch id: {error}")))?;
    let request_id = Uuid::parse_str(&row.request_id).map_err(|error| {
        AppError::infrastructure(format!("invalid dispatch request id: {error}"))
    })?;
    let request = serde_json::from_str(&row.request_json)
        .map_err(|error| AppError::infrastructure(format!("invalid dispatch request: {error}")))?;

    Ok(DispatchJobResponse {
        id,
        request_id,
        model: row.model,
        request,
        timeout_at: row.timeout_at,
    })
}
