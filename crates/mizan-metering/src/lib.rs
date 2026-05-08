use mizan_providers::TokenUsage;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    pub request_id: Uuid,
    pub user_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
    pub provider_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub model: String,
    pub usage: TokenUsage,
    pub status_code: u16,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct UsageChargeInput {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl From<TokenUsage> for UsageChargeInput {
    fn from(usage: TokenUsage) -> Self {
        Self {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
        }
    }
}
