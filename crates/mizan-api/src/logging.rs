use axum::http::StatusCode;
use mizan_core::{AppError, AppResult, DatabaseBackend};
use serde::Serialize;
use sqlx::{AnyPool, query};
use uuid::Uuid;

use crate::utils::{prepare_sql, unix_timestamp_string};

const UUID_TEXT_BUFFER_LEN: usize = uuid::fmt::Urn::LENGTH;

#[derive(Debug, Clone, Copy)]
struct UuidText {
    buf: [u8; UUID_TEXT_BUFFER_LEN],
    len: usize,
}

impl UuidText {
    fn new(value: Uuid) -> Self {
        let mut buf = Uuid::encode_buffer();
        let len = {
            let encoded = value.hyphenated().encode_lower(&mut buf);
            encoded.len()
        };

        Self { buf, len }
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..self.len]).expect("uuid text buffer must be valid utf8")
    }
}

#[derive(Debug, Clone)]
pub struct RequestLogInput {
    pub request_id: Uuid,
    pub user_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
    pub provider_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub method: String,
    pub path: String,
    pub route: Option<String>,
    pub provider: Option<String>,
    pub status_code: StatusCode,
    pub latency_ms: u64,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AdminAuditInput {
    pub actor_user_id: Option<Uuid>,
    pub action: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub payload_json: Option<String>,
}

pub fn error_code_from_app_error(error: &AppError) -> String {
    error.public_code().to_string()
}

pub fn serialize_payload(payload: impl Serialize) -> Option<String> {
    serde_json::to_string(&payload).ok()
}

pub async fn record_request_log(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    input: &RequestLogInput,
) -> AppResult<()> {
    let now = unix_timestamp_string();
    let status_code = i64::from(u16::from(input.status_code));
    let latency_ms = i64::try_from(input.latency_ms).map_err(|error| {
        AppError::infrastructure(format!("request_log.latency_ms exceeds i64 range: {error}"))
    })?;
    let id = UuidText::new(Uuid::now_v7());
    let request_id = UuidText::new(input.request_id);
    let user_id = input.user_id.map(UuidText::new);
    let api_key_id = input.api_key_id.map(UuidText::new);
    let provider_id = input.provider_id.map(UuidText::new);
    let route_id = input.route_id.map(UuidText::new);

    query(&prepare_sql(
        database_backend,
        "INSERT INTO request_logs (
            id,
            request_id,
            user_id,
            api_key_id,
            provider_id,
            route_id,
            method,
            path,
            route,
            provider,
            status_code,
            latency_ms,
            error_code,
            created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(id.as_str())
    .bind(request_id.as_str())
    .bind(user_id.as_ref().map(UuidText::as_str))
    .bind(api_key_id.as_ref().map(UuidText::as_str))
    .bind(provider_id.as_ref().map(UuidText::as_str))
    .bind(route_id.as_ref().map(UuidText::as_str))
    .bind(&input.method)
    .bind(&input.path)
    .bind(input.route.as_ref())
    .bind(input.provider.as_ref())
    .bind(status_code)
    .bind(latency_ms)
    .bind(&input.error_code)
    .bind(&now)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(format!("cannot insert request log: {error}")))?;

    Ok(())
}

pub async fn record_admin_audit(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    input: &AdminAuditInput,
) -> AppResult<()> {
    let now = unix_timestamp_string();
    let id = UuidText::new(Uuid::now_v7());
    let actor_user_id = input.actor_user_id.map(UuidText::new);

    query(&prepare_sql(
        database_backend,
        "INSERT INTO admin_audit_logs (
            id,
            actor_user_id,
            action,
            entity_type,
            entity_id,
            payload_json,
            created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(id.as_str())
    .bind(actor_user_id.as_ref().map(UuidText::as_str))
    .bind(&input.action)
    .bind(&input.entity_type)
    .bind(&input.entity_id)
    .bind(&input.payload_json)
    .bind(&now)
    .execute(database)
    .await
    .map_err(|error| AppError::infrastructure(format!("cannot insert admin audit log: {error}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;
    use sqlx::query_scalar;

    #[derive(Debug, Serialize)]
    struct SamplePayload {
        id: u32,
        note: String,
    }

    #[test]
    fn uuid_text_renders_hyphenated_uuid() {
        let text = UuidText::new(Uuid::nil());

        assert_eq!(text.as_str(), "00000000-0000-0000-0000-000000000000");
    }

    async fn test_database() -> AnyPool {
        storage::connect_and_migrate("sqlite::memory:", true, 1)
            .await
            .expect("create memory sqlite")
    }

    #[tokio::test]
    async fn request_log_can_be_written_with_optional_error_code() {
        let database = test_database().await;
        let request_id = Uuid::now_v7();

        record_request_log(
            &database,
            DatabaseBackend::Sqlite,
            &RequestLogInput {
                request_id,
                user_id: None,
                api_key_id: None,
                provider_id: None,
                route_id: None,
                method: "POST".to_owned(),
                path: "/v1/chat/completions".to_owned(),
                route: Some("mizan/gpt-4o-mini".to_owned()),
                provider: Some("openai".to_owned()),
                status_code: StatusCode::OK,
                latency_ms: 12,
                error_code: Some("ok".to_owned()),
            },
        )
        .await
        .expect("insert request log");

        let row_count: i64 = query_scalar("SELECT COUNT(*) FROM request_logs")
            .fetch_one(&database)
            .await
            .expect("count request logs");

        assert_eq!(row_count, 1);
    }

    #[tokio::test]
    async fn admin_audit_can_be_written_with_redacted_payload() {
        let database = test_database().await;
        let payload = SamplePayload {
            id: 7,
            note: "provider=openai".to_owned(),
        };

        let payload_json = serialize_payload(&payload).expect("payload json");
        assert!(payload_json.contains(r#""id":7"#));

        record_admin_audit(
            &database,
            DatabaseBackend::Sqlite,
            &AdminAuditInput {
                actor_user_id: None,
                action: "audit.test".to_owned(),
                entity_type: "provider".to_owned(),
                entity_id: Some("123e4567-e89b-12d3-a456-426614174000".to_owned()),
                payload_json: Some(payload_json),
            },
        )
        .await
        .expect("insert audit log");

        let row_count: i64 = query_scalar("SELECT COUNT(*) FROM admin_audit_logs")
            .fetch_one(&database)
            .await
            .expect("count audit logs");

        assert_eq!(row_count, 1);
    }
}
