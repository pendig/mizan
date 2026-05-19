use mizan_core::{AppResult, RequestContextBuilder};
use mizan_providers::{
    ChatCompletionStream, ChatMessage, ChatRequest, ChatResponse, OpenAiCompatibleProvider,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type ChatProxyStream = ChatCompletionStream;

pub use mizan_providers::{ChatMessage, ChatRequest, ChatResponse};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatProxyConfig {
    pub base_url: String,
    pub api_key: String,
    pub provider_name: Option<String>,
}

impl ChatProxyConfig {
    pub fn with_provider_name(mut self, provider_name: impl Into<String>) -> Self {
        self.provider_name = Some(provider_name.into());
        self
    }
}

pub fn chat_completion_request(
    model: impl Into<String>,
    prompt: impl Into<String>,
) -> ChatRequest {
    ChatRequest {
        model: model.into(),
        messages: vec![ChatMessage {
            role: "user".to_owned(),
            content: prompt.into(),
        }],
        stream: false,
        max_tokens: None,
    }
}

pub fn chat_completion_request_with_messages(
    model: impl Into<String>,
    messages: Vec<ChatMessage>,
    stream: bool,
    max_tokens: Option<u64>,
) -> ChatRequest {
    ChatRequest {
        model: model.into(),
        messages,
        stream,
        max_tokens,
    }
}

fn provider_name_for(base_url: &str, provider_name: Option<String>) -> String {
    provider_name.unwrap_or_else(|| {
        if base_url.to_ascii_lowercase().contains("openai") {
            "openai".to_owned()
        } else {
            "openai-compatible".to_owned()
        }
    })
}

pub async fn send_chat_completion(
    config: &ChatProxyConfig,
    request: ChatRequest,
) -> AppResult<ChatResponse> {
    let provider = OpenAiCompatibleProvider::new(
        provider_name_for(&config.base_url, config.provider_name.clone()),
        &config.base_url,
        &config.api_key,
    );

    let request_id = Uuid::now_v7();
    let context = RequestContextBuilder::default()
        .request_id(request_id)
        .trace_id(request_id)
        .streaming(request.stream)
        .build();

    provider.chat_completions(&context, request).await
}

pub async fn send_chat_completion_stream(
    config: &ChatProxyConfig,
    request: ChatRequest,
) -> AppResult<ChatProxyStream> {
    let provider = OpenAiCompatibleProvider::new(
        provider_name_for(&config.base_url, config.provider_name.clone()),
        &config.base_url,
        &config.api_key,
    );

    let request_id = Uuid::now_v7();
    let context = RequestContextBuilder::default()
        .request_id(request_id)
        .trace_id(request_id)
        .streaming(request.stream)
        .build();

    provider.chat_completions_stream(&context, request).await
}
