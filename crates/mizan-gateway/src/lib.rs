use mizan_core::{AppResult, RequestContext};
use mizan_providers::{ChatRequest, ChatResponse, ProviderAdapter};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayHealth {
    pub status: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct Gateway;

impl Gateway {
    pub fn new() -> Self {
        Self
    }

    pub fn health(&self) -> GatewayHealth {
        GatewayHealth { status: "ok" }
    }

    pub async fn chat_completions(
        &self,
        context: &RequestContext,
        provider: &dyn ProviderAdapter,
        request: ChatRequest,
    ) -> AppResult<ChatResponse> {
        provider.chat_completions(context, request).await
    }
}
