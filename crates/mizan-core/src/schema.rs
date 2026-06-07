use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Shared database tables used across API, gateway, limits, and wallet modules.
pub mod tables {
    pub const USERS: &str = "users";
    pub const SESSIONS: &str = "sessions";
    pub const API_KEYS: &str = "api_keys";
    pub const PROVIDER_CONNECTIONS: &str = "provider_connections";
    pub const MODEL_ROUTES: &str = "model_routes";
    pub const WALLETS: &str = "wallets";
    pub const CREDIT_LEDGER: &str = "credit_ledger";
    pub const USAGE_EVENTS: &str = "usage_events";
    pub const REQUEST_LOGS: &str = "request_logs";
    pub const ADMIN_AUDIT_LOGS: &str = "admin_audit_logs";
    pub const DAEMON_NODES: &str = "daemon_nodes";
}

pub fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}

pub fn i64_to_bool(value: i64) -> bool {
    value != 0
}

pub fn i8_to_bool(value: i8) -> bool {
    value != 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub role: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub session_token_hash: String,
    pub expires_at: Option<String>,
    pub revoked: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub key_hash: String,
    pub label: Option<String>,
    pub revoked: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConnectionRecord {
    pub id: Uuid,
    pub name: String,
    pub provider_type: String,
    pub auth_mode: String,
    pub auth_config_json: Option<String>,
    pub base_url: String,
    pub api_key_encrypted: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRouteRecord {
    pub id: Uuid,
    pub provider_connection_id: Uuid,
    pub public_model: String,
    pub upstream_model: String,
    pub max_tokens: Option<i64>,
    pub pricing_input_per_1m_tokens: i64,
    pub pricing_output_per_1m_tokens: i64,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletRecord {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub balance_microcredits: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditLedgerRecord {
    pub id: Uuid,
    pub wallet_id: Uuid,
    pub request_id: Option<Uuid>,
    pub request_delta_microcredits: i64,
    pub balance_after_microcredits: i64,
    pub reason: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEventRecord {
    pub id: Uuid,
    pub request_id: String,
    pub user_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
    pub provider_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub model: String,
    pub usage_prompt_tokens: i64,
    pub usage_completion_tokens: i64,
    pub usage_total_tokens: i64,
    pub usage_estimated: bool,
    pub status_code: i64,
    pub latency_ms: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLogRecord {
    pub id: Uuid,
    pub request_id: String,
    pub user_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
    pub provider_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub method: String,
    pub path: String,
    pub route: Option<String>,
    pub provider: Option<String>,
    pub status_code: i64,
    pub latency_ms: i64,
    pub error_code: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminAuditLogRecord {
    pub id: Uuid,
    pub actor_user_id: Option<Uuid>,
    pub action: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub payload_json: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonNodeRecord {
    pub id: Uuid,
    pub host_user_id: Option<Uuid>,
    pub label: Option<String>,
    pub hostname: Option<String>,
    pub public_key: Option<String>,
    pub token_hash: String,
    pub status: String,
    pub revoked: bool,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_helper_roundtrip() {
        assert_eq!(bool_to_i64(true), 1);
        assert_eq!(bool_to_i64(false), 0);
        assert!(i64_to_bool(1));
        assert!(!i64_to_bool(0));
        assert!(i8_to_bool(1));
        assert!(!i8_to_bool(0));
    }
}
