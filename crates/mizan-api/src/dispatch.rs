use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use mizan_core::{AppError, DatabaseBackend, ErrorEnvelope};
use mizan_providers::{ChatRequest, ChatResponse};
use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, query, query_as};
use tokio::time::sleep;
use tracing::warn;
use uuid::Uuid;

use crate::AppState;
use crate::daemon_nodes::{DaemonNodeIdentity, EligibleDaemonNode};
use crate::utils::{from_app_error, now_utc_epoch_seconds, prepare_sql, unix_timestamp_string};

type DispatchHttpResult<T> = Result<T, (StatusCode, Json<ErrorEnvelope>)>;

pub const STATUS_ACCEPTED: &str = "accepted";
pub const STATUS_LEASED: &str = "leased";
pub const STATUS_SUCCEEDED: &str = "succeeded";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_TIMED_OUT: &str = "timed_out";

#[derive(Debug, Clone)]
pub struct DispatchJobInput {
    pub request_id: Uuid,
    pub node_id: Uuid,
    pub user_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
    pub model: String,
    pub request: ChatRequest,
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchJobLeaseResponse {
    pub id: Uuid,
    pub request_id: Uuid,
    pub model: String,
    pub request: ChatRequest,
    pub deadline_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DispatchJobLeaseEnvelope {
    pub data: Option<DispatchJobLeaseResponse>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DispatchJobCompleteRequest {
    pub status: String,
    #[serde(default)]
    pub response: Option<ChatResponse>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DispatchJobStatusResponse {
    pub id: Uuid,
    pub status: String,
}

#[derive(Debug, Clone)]
pub enum DispatchJobResult {
    Succeeded(ChatResponse),
    Failed {
        error_code: Option<String>,
        error_message: String,
    },
    TimedOut,
}

#[derive(Debug, Clone)]
struct DispatchJobRow {
    id: String,
    request_id: String,
    model: String,
    request_json: String,
    deadline_at: String,
}

pub async fn lease_next_dispatch_job(
    State(state): State<AppState>,
    Extension(identity): Extension<DaemonNodeIdentity>,
) -> DispatchHttpResult<Json<DispatchJobLeaseEnvelope>> {
    let data = lease_next_job_for_node(&state.database, state.database_backend(), identity.node_id)
        .await
        .map_err(from_app_error)?;

    Ok(Json(DispatchJobLeaseEnvelope { data }))
}

pub async fn complete_dispatch_job(
    State(state): State<AppState>,
    Extension(identity): Extension<DaemonNodeIdentity>,
    Path(id): Path<Uuid>,
    Json(payload): Json<DispatchJobCompleteRequest>,
) -> DispatchHttpResult<Json<DispatchJobStatusResponse>> {
    let status = complete_job(
        &state.database,
        state.database_backend(),
        identity.node_id,
        id,
        payload,
    )
    .await
    .map_err(from_app_error)?;

    Ok(Json(DispatchJobStatusResponse { id, status }))
}

pub async fn create_dispatch_job(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    input: DispatchJobInput,
) -> Result<Uuid, AppError> {
    let id = Uuid::now_v7();
    let now = unix_timestamp_string();
    let deadline_at = now_utc_epoch_seconds()
        .saturating_add(i64::from(input.timeout_seconds.max(1)))
        .to_string();
    let request_json = serde_json::to_string(&input.request).map_err(|error| {
        AppError::infrastructure(format!("dispatch request encode failed: {error}"))
    })?;

    query(&prepare_sql(
        database_backend,
        "INSERT INTO dispatch_jobs (
             id,
             request_id,
             node_id,
             user_id,
             api_key_id,
             model,
             status,
             request_json,
             deadline_at,
             created_at,
             updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(id.to_string())
    .bind(input.request_id.to_string())
    .bind(input.node_id.to_string())
    .bind(input.user_id.map(|value| value.to_string()))
    .bind(input.api_key_id.map(|value| value.to_string()))
    .bind(input.model)
    .bind(STATUS_ACCEPTED)
    .bind(request_json)
    .bind(deadline_at)
    .bind(&now)
    .bind(&now)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    Ok(id)
}

pub async fn dispatch_to_daemon_node(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    node: &EligibleDaemonNode,
    input: DispatchJobInput,
) -> Result<DispatchJobResult, AppError> {
    let timeout_seconds = input.timeout_seconds.max(1);
    let job_id = create_dispatch_job(database, database_backend, input).await?;

    let result = match wait_for_dispatch_result(
        database,
        database_backend,
        job_id,
        Duration::from_secs(u64::from(timeout_seconds)),
    )
    .await?
    {
        DispatchJobResult::TimedOut => {
            mark_job_timed_out(database, database_backend, job_id).await?;
            Ok(DispatchJobResult::TimedOut)
        }
        result => Ok(result),
    };

    if let Err(error) = &result {
        warn!(
            node_id = %node.id,
            job_id = %job_id,
            error = %error,
            "daemon dispatch failed"
        );
    }

    result
}

pub async fn lease_next_job_for_node(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    node_id: Uuid,
) -> Result<Option<DispatchJobLeaseResponse>, AppError> {
    mark_expired_jobs_timed_out(database, database_backend).await?;
    let now = unix_timestamp_string();

    let rows = query_as::<_, (String, String, String, String, String)>(&prepare_sql(
        database_backend,
        "SELECT id, request_id, model, request_json, deadline_at
         FROM dispatch_jobs
         WHERE node_id = ?
           AND status = ?
           AND deadline_at >= ?
         ORDER BY created_at ASC
         LIMIT 5",
    ))
    .bind(node_id.to_string())
    .bind(STATUS_ACCEPTED)
    .bind(&now)
    .fetch_all(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    for row in rows {
        let row = DispatchJobRow {
            id: row.0,
            request_id: row.1,
            model: row.2,
            request_json: row.3,
            deadline_at: row.4,
        };
        let updated = query(&prepare_sql(
            database_backend,
            "UPDATE dispatch_jobs
             SET status = ?, leased_at = ?, updated_at = ?
             WHERE id = ? AND node_id = ? AND status = ?",
        ))
        .bind(STATUS_LEASED)
        .bind(&now)
        .bind(&now)
        .bind(&row.id)
        .bind(node_id.to_string())
        .bind(STATUS_ACCEPTED)
        .execute(database)
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?;

        if updated.rows_affected() != 1 {
            continue;
        }

        let id = Uuid::parse_str(&row.id).map_err(|error| {
            AppError::infrastructure(format!("stored dispatch id is invalid: {error}"))
        })?;
        let request_id = Uuid::parse_str(&row.request_id).map_err(|error| {
            AppError::infrastructure(format!("stored dispatch request id is invalid: {error}"))
        })?;
        let request = serde_json::from_str::<ChatRequest>(&row.request_json).map_err(|error| {
            AppError::infrastructure(format!("stored dispatch request is invalid: {error}"))
        })?;

        return Ok(Some(DispatchJobLeaseResponse {
            id,
            request_id,
            model: row.model,
            request,
            deadline_at: row.deadline_at,
        }));
    }

    Ok(None)
}

pub async fn complete_job(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    node_id: Uuid,
    job_id: Uuid,
    payload: DispatchJobCompleteRequest,
) -> Result<String, AppError> {
    let status = match payload.status.trim() {
        STATUS_SUCCEEDED => STATUS_SUCCEEDED,
        STATUS_FAILED => STATUS_FAILED,
        _ => {
            return Err(AppError::invalid_config(
                "dispatch_job.status",
                "status must be succeeded or failed",
            ));
        }
    };

    if status == STATUS_SUCCEEDED && payload.response.is_none() {
        return Err(AppError::invalid_config(
            "dispatch_job.response",
            "response is required when status is succeeded",
        ));
    }

    let response_json = payload
        .response
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| {
            AppError::infrastructure(format!("dispatch response encode failed: {error}"))
        })?;
    let now = unix_timestamp_string();
    let error_message = payload
        .error_message
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    let updated = query(&prepare_sql(
        database_backend,
        "UPDATE dispatch_jobs
         SET status = ?,
             response_json = ?,
             error_code = ?,
             error_message = ?,
             completed_at = ?,
             updated_at = ?
         WHERE id = ? AND node_id = ? AND status = ?",
    ))
    .bind(status)
    .bind(response_json)
    .bind(payload.error_code)
    .bind(error_message)
    .bind(&now)
    .bind(&now)
    .bind(job_id.to_string())
    .bind(node_id.to_string())
    .bind(STATUS_LEASED)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    if updated.rows_affected() != 1 {
        return Err(AppError::NotFound(
            "leased dispatch job not found".to_owned(),
        ));
    }

    Ok(status.to_owned())
}

pub async fn wait_for_dispatch_result(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    job_id: Uuid,
    timeout_duration: Duration,
) -> Result<DispatchJobResult, AppError> {
    let started_at = Instant::now();

    loop {
        if let Some(result) = fetch_terminal_result(database, database_backend, job_id).await? {
            return Ok(result);
        }

        if started_at.elapsed() >= timeout_duration {
            return Ok(DispatchJobResult::TimedOut);
        }

        sleep(Duration::from_millis(100)).await;
    }
}

async fn fetch_terminal_result(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    job_id: Uuid,
) -> Result<Option<DispatchJobResult>, AppError> {
    let row =
        query_as::<_, (String, Option<String>, Option<String>, Option<String>)>(&prepare_sql(
            database_backend,
            "SELECT status, response_json, error_code, error_message
         FROM dispatch_jobs
         WHERE id = ?",
        ))
        .bind(job_id.to_string())
        .fetch_optional(database)
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?;

    let Some((status, response_json, error_code, error_message)) = row else {
        return Err(AppError::NotFound("dispatch job not found".to_owned()));
    };

    match status.as_str() {
        STATUS_SUCCEEDED => {
            let raw = response_json.ok_or_else(|| {
                AppError::infrastructure("dispatch job succeeded without response_json")
            })?;
            let response = serde_json::from_str::<ChatResponse>(&raw).map_err(|error| {
                AppError::infrastructure(format!("stored dispatch response is invalid: {error}"))
            })?;
            Ok(Some(DispatchJobResult::Succeeded(response)))
        }
        STATUS_FAILED => Ok(Some(DispatchJobResult::Failed {
            error_code,
            error_message: error_message.unwrap_or_else(|| "daemon job failed".to_owned()),
        })),
        STATUS_TIMED_OUT => Ok(Some(DispatchJobResult::TimedOut)),
        _ => Ok(None),
    }
}

async fn mark_job_timed_out(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    job_id: Uuid,
) -> Result<(), AppError> {
    let now = unix_timestamp_string();
    query(&prepare_sql(
        database_backend,
        "UPDATE dispatch_jobs
         SET status = ?, error_code = ?, error_message = ?, completed_at = ?, updated_at = ?
         WHERE id = ? AND status IN (?, ?)",
    ))
    .bind(STATUS_TIMED_OUT)
    .bind("timeout")
    .bind("daemon dispatch timed out")
    .bind(&now)
    .bind(&now)
    .bind(job_id.to_string())
    .bind(STATUS_ACCEPTED)
    .bind(STATUS_LEASED)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    Ok(())
}

async fn mark_expired_jobs_timed_out(
    database: &AnyPool,
    database_backend: DatabaseBackend,
) -> Result<(), AppError> {
    let now = unix_timestamp_string();
    query(&prepare_sql(
        database_backend,
        "UPDATE dispatch_jobs
         SET status = ?, error_code = ?, error_message = ?, completed_at = ?, updated_at = ?
         WHERE status IN (?, ?) AND deadline_at < ?",
    ))
    .bind(STATUS_TIMED_OUT)
    .bind("timeout")
    .bind("daemon dispatch timed out")
    .bind(&now)
    .bind(&now)
    .bind(STATUS_ACCEPTED)
    .bind(STATUS_LEASED)
    .bind(&now)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_nodes::{HEALTH_STATUS_HEALTHY, STATUS_ACTIVE};
    use crate::storage;
    use sqlx::query;

    async fn sqlite_test_database() -> AnyPool {
        storage::connect_and_migrate("sqlite::memory:", true, 1)
            .await
            .expect("create sqlite test database")
    }

    async fn seed_node(database: &AnyPool) -> Uuid {
        let node_id = Uuid::now_v7();
        let user_id = Uuid::now_v7();
        let now = unix_timestamp_string();
        query("INSERT INTO users (id, email, password_hash, role, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(user_id.to_string())
            .bind(format!("{user_id}@example.test"))
            .bind("hash")
            .bind("admin")
            .bind(&now)
            .bind(&now)
            .execute(database)
            .await
            .expect("insert user");
        query(
            "INSERT INTO daemon_nodes (
                 id, host_user_id, token_hash, status, revoked, disabled, last_seen_at,
                 provider_family, model_ids_json, max_concurrency, health_status, created_at, updated_at
             ) VALUES (?, ?, ?, ?, 0, 0, ?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(node_id.to_string())
        .bind(user_id.to_string())
        .bind(format!("hash-{node_id}"))
        .bind(STATUS_ACTIVE)
        .bind(&now)
        .bind("openai-compatible")
        .bind(r#"["llama3.1"]"#)
        .bind(HEALTH_STATUS_HEALTHY)
        .bind(&now)
        .bind(&now)
        .execute(database)
        .await
        .expect("insert daemon node");
        node_id
    }

    fn test_request() -> ChatRequest {
        ChatRequest {
            model: "llama3.1".to_owned(),
            messages: Vec::new(),
            stream: false,
            max_tokens: Some(8),
        }
    }

    #[tokio::test]
    async fn daemon_can_lease_and_complete_dispatch_job() {
        let database = sqlite_test_database().await;
        let node_id = seed_node(&database).await;
        let request_id = Uuid::now_v7();
        let job_id = create_dispatch_job(
            &database,
            DatabaseBackend::Sqlite,
            DispatchJobInput {
                request_id,
                node_id,
                user_id: None,
                api_key_id: None,
                model: "llama3.1".to_owned(),
                request: test_request(),
                timeout_seconds: 5,
            },
        )
        .await
        .expect("create dispatch job");

        let leased = lease_next_job_for_node(&database, DatabaseBackend::Sqlite, node_id)
            .await
            .expect("lease job")
            .expect("job should exist");
        assert_eq!(leased.id, job_id);
        assert_eq!(leased.request_id, request_id);

        complete_job(
            &database,
            DatabaseBackend::Sqlite,
            node_id,
            job_id,
            DispatchJobCompleteRequest {
                status: STATUS_SUCCEEDED.to_owned(),
                response: Some(ChatResponse {
                    provider: "mizan-daemon".to_owned(),
                    model: "llama3.1".to_owned(),
                    content: "pong".to_owned(),
                    usage: None,
                }),
                error_code: None,
                error_message: None,
            },
        )
        .await
        .expect("complete job");

        match fetch_terminal_result(&database, DatabaseBackend::Sqlite, job_id)
            .await
            .expect("fetch result")
            .expect("terminal result")
        {
            DispatchJobResult::Succeeded(response) => assert_eq!(response.content, "pong"),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn expired_jobs_are_marked_timed_out_before_lease() {
        let database = sqlite_test_database().await;
        let node_id = seed_node(&database).await;
        create_dispatch_job(
            &database,
            DatabaseBackend::Sqlite,
            DispatchJobInput {
                request_id: Uuid::now_v7(),
                node_id,
                user_id: None,
                api_key_id: None,
                model: "llama3.1".to_owned(),
                request: test_request(),
                timeout_seconds: 1,
            },
        )
        .await
        .expect("create dispatch job");

        sleep(Duration::from_secs(2)).await;
        let leased = lease_next_job_for_node(&database, DatabaseBackend::Sqlite, node_id)
            .await
            .expect("lease job");

        assert!(leased.is_none());
    }
}
