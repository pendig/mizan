use async_trait::async_trait;
use mizan_core::{AppResult, RequestContext};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub provider: String,
    pub model: String,
    pub content: String,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModel {
    pub id: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub estimated: bool,
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn name(&self) -> &'static str;

    async fn chat_completions(
        &self,
        context: &RequestContext,
        request: ChatRequest,
    ) -> AppResult<ChatResponse>;

    async fn models(&self, context: &RequestContext) -> AppResult<Vec<ProviderModel>>;
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    name: &'static str,
}

impl OpenAiCompatibleProvider {
    pub fn new(name: &'static str) -> Self {
        Self { name }
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiCompatibleProvider {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn chat_completions(
        &self,
        _context: &RequestContext,
        request: ChatRequest,
    ) -> AppResult<ChatResponse> {
        Ok(ChatResponse {
            provider: self.name.to_owned(),
            model: request.model,
            content: String::new(),
            usage: None,
        })
    }

    async fn models(&self, _context: &RequestContext) -> AppResult<Vec<ProviderModel>> {
        Ok(Vec::new())
    }
}
