use async_trait::async_trait;
use futures_util::stream::BoxStream;
use futures_util::{StreamExt, stream};
use mizan_core::{AppError, AppResult, RequestContext};
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
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
pub struct ChatStreamChunk {
    pub index: usize,
    pub delta: String,
    pub finish_reason: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub status: &'static str,
    pub latency_ms: Option<u64>,
}

pub type ChatCompletionStream = BoxStream<'static, ChatStreamChunk>;

impl Default for ProviderHealth {
    fn default() -> Self {
        Self {
            status: "unknown",
            latency_ms: None,
        }
    }
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn name(&self) -> &str;

    async fn chat_completions(
        &self,
        context: &RequestContext,
        request: ChatRequest,
    ) -> AppResult<ChatResponse>;

    async fn chat_completions_stream(
        &self,
        context: &RequestContext,
        request: ChatRequest,
    ) -> AppResult<ChatCompletionStream> {
        let response = self.chat_completions(context, request).await?;
        Ok(stream::iter([ChatStreamChunk {
            index: 0,
            delta: response.content,
            finish_reason: Some("stop".to_owned()),
            usage: response.usage,
        }])
        .boxed())
    }

    async fn models(&self, context: &RequestContext) -> AppResult<Vec<ProviderModel>>;

    async fn health(&self, _context: &RequestContext) -> AppResult<ProviderHealth> {
        Ok(ProviderHealth::default())
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    name: String,
    base_url: String,
    api_key: String,
    client: Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into().trim().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            client: Client::new(),
        }
    }

    fn chat_completion_url(&self) -> String {
        if self.base_url.ends_with("/v1") {
            format!("{}/chat/completions", self.base_url)
        } else {
            format!("{}/v1/chat/completions", self.base_url)
        }
    }

    fn with_api_request_headers(
        &self,
        request: reqwest::RequestBuilder,
        request_id: &str,
    ) -> reqwest::RequestBuilder {
        request
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header("x-request-id", request_id)
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    async fn chat_completions(
        &self,
        context: &RequestContext,
        request: ChatRequest,
    ) -> AppResult<ChatResponse> {
        let upstream_response = self
            .send_chat_completion_request(context, &request, false)
            .await?
            .text()
            .await
            .map_err(|error| {
                AppError::infrastructure(format!(
                    "upstream chat completion response read failed: {error}"
                ))
            })?;

        let parsed =
            parse_chat_completion_response(&upstream_response, request.model, self.name.clone())?;

        Ok(parsed)
    }

    async fn chat_completions_stream(
        &self,
        context: &RequestContext,
        request: ChatRequest,
    ) -> AppResult<ChatCompletionStream> {
        let upstream_response = self
            .send_chat_completion_request(context, &request, true)
            .await?
            .text()
            .await
            .map_err(|error| {
                AppError::infrastructure(format!(
                    "upstream chat completion stream read failed: {error}"
                ))
            })?;

        let parsed = parse_stream_events(&upstream_response)?;

        Ok(stream::iter(parsed).boxed())
    }

    async fn models(&self, _context: &RequestContext) -> AppResult<Vec<ProviderModel>> {
        Ok(Vec::new())
    }

    async fn health(&self, _context: &RequestContext) -> AppResult<ProviderHealth> {
        Ok(ProviderHealth {
            status: "ok",
            latency_ms: None,
        })
    }
}

impl OpenAiCompatibleProvider {
    async fn send_chat_completion_request(
        &self,
        context: &RequestContext,
        request: &ChatRequest,
        stream: bool,
    ) -> AppResult<reqwest::Response> {
        let payload = OpenAiChatCompletionPayload {
            model: request.model.clone(),
            messages: request.messages.clone(),
            stream,
        };

        let response = self
            .with_api_request_headers(
                self.client.post(self.chat_completion_url()).json(&payload),
                &context.request_id.to_string(),
            )
            .send()
            .await
            .map_err(|error| {
                AppError::infrastructure(format!("upstream request failed: {error}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read upstream body>".to_owned());

            return Err(AppError::provider(format!(
                "upstream provider returned status={status} body={body}"
            )));
        }

        Ok(response)
    }
}

fn parse_chat_completion_response(
    raw_body: &str,
    model: String,
    provider_name: String,
) -> AppResult<ChatResponse> {
    let response: OpenAiChatCompletionResponse =
        serde_json::from_str(raw_body).map_err(|error| {
            AppError::infrastructure(format!(
                "failed to decode upstream chat completion response: {error}"
            ))
        })?;

    let mut content = String::new();
    if let Some(first_choice) = response.choices.into_iter().next() {
        content = first_choice
            .message
            .and_then(|message| message.content)
            .unwrap_or_default();
    }

    Ok(ChatResponse {
        provider: provider_name,
        model: response.model.unwrap_or(model),
        content,
        usage: response.usage.as_ref().map(normalize_usage),
    })
}

fn parse_stream_events(raw_body: &str) -> AppResult<Vec<ChatStreamChunk>> {
    let mut chunks = Vec::new();
    let mut event_data = String::new();

    for raw_line in raw_body.lines() {
        let line = raw_line.trim_end();

        if line.is_empty() {
            if event_data.trim().is_empty() {
                continue;
            }

            append_stream_event(&mut chunks, &event_data)?;
            event_data.clear();
            continue;
        }

        let trimmed = line.trim_start();
        if trimmed.starts_with(':') {
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("data:") {
            event_data.push_str(value.trim_start());
        }
    }

    if !event_data.trim().is_empty() {
        append_stream_event(&mut chunks, &event_data)?;
    }

    if chunks.is_empty() {
        return Err(AppError::provider("upstream stream response was empty"));
    }

    Ok(chunks)
}

fn append_stream_event(chunks: &mut Vec<ChatStreamChunk>, raw_event_data: &str) -> AppResult<()> {
    let payload = raw_event_data.trim();
    if payload == "[DONE]" {
        return Ok(());
    }

    let chunk: OpenAiChatCompletionChunk = serde_json::from_str(payload)
        .map_err(|error| AppError::provider(format!("invalid upstream stream payload: {error}")))?;

    for choice in chunk.choices {
        let delta = choice.delta.content.unwrap_or_default();
        if delta.is_empty() && choice.finish_reason.is_none() {
            continue;
        }

        chunks.push(ChatStreamChunk {
            index: choice.index,
            delta,
            finish_reason: choice.finish_reason,
            usage: chunk.usage.as_ref().map(normalize_usage),
        });
    }

    if chunks.is_empty()
        && let Some(usage) = chunk.usage
    {
        chunks.push(ChatStreamChunk {
            index: 0,
            delta: String::new(),
            finish_reason: None,
            usage: Some(normalize_usage(&usage)),
        });
    }

    Ok(())
}

fn normalize_usage(raw_usage: &OpenAiTokenUsage) -> TokenUsage {
    TokenUsage {
        prompt_tokens: raw_usage.prompt_tokens,
        completion_tokens: raw_usage.completion_tokens,
        total_tokens: raw_usage.total_tokens,
        estimated: raw_usage.estimated,
    }
}

#[derive(Debug, Serialize)]
struct OpenAiChatCompletionPayload {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatCompletionResponse {
    pub model: Option<String>,
    pub choices: Vec<OpenAiChoice>,
    pub usage: Option<OpenAiTokenUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    pub message: Option<OpenAiChatMessage>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiChatMessage {
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatCompletionChunk {
    pub choices: Vec<OpenAiChunkChoice>,
    pub usage: Option<OpenAiTokenUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChunkChoice {
    pub index: usize,
    pub delta: OpenAiStreamMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamMessage {
    pub content: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiTokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub estimated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_non_stream_response_with_usage() {
        let raw_response = r#"
        {
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "model": "gpt-4o-mini",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "hello"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 2,
                "total_tokens": 12,
                "estimated": false
            }
        }
        "#;

        let parsed = parse_chat_completion_response(
            raw_response,
            "mizan/public-gpt".to_owned(),
            "openai".to_owned(),
        )
        .expect("parse completion response");

        assert_eq!(parsed.model, "gpt-4o-mini");
        assert_eq!(parsed.content, "hello");
        assert!(parsed.usage.is_some());
        assert_eq!(parsed.usage.unwrap().total_tokens, 12);
    }

    #[test]
    fn parse_stream_events_extracts_sse_chunks() {
        let raw_stream = "".to_string()
            + "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"finish_reason\":null}] }\n"
            + "\n"
            + "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\" world\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2,\"total_tokens\":3,\"estimated\":false}}\n\n"
            + "data: [DONE]\n";

        let chunks = parse_stream_events(&raw_stream).expect("parse stream events");
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].delta, "Hello");
        assert_eq!(chunks[1].delta, " world");
    }
}
