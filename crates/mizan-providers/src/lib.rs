use std::collections::VecDeque;
use std::sync::LazyLock;

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use futures_util::{StreamExt, stream};
use mizan_core::{AppError, AppResult, RequestContext, redact_for_logs};
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub max_tokens: Option<u64>,
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

pub type ChatCompletionStream = BoxStream<'static, AppResult<ChatStreamChunk>>;

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
        Ok(stream::iter([Ok(ChatStreamChunk {
            index: 0,
            delta: response.content,
            finish_reason: Some("stop".to_owned()),
            usage: response.usage,
        })])
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
    api_key: Option<String>,
}

impl OpenAiCompatibleProvider {
    fn http_client() -> &'static Client {
        static CLIENT: LazyLock<Client> = LazyLock::new(Client::new);
        &CLIENT
    }

    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self::with_optional_api_key(name, base_url, Some(api_key.into()))
    }

    pub fn with_optional_api_key(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        let api_key = api_key
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        Self {
            name: name.into(),
            base_url: base_url.into().trim().trim_end_matches('/').to_string(),
            api_key,
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
        let request = request
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .header("x-request-id", request_id);

        if let Some(api_key) = &self.api_key {
            request.header(AUTHORIZATION, format!("Bearer {api_key}"))
        } else {
            request
        }
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
            .await?;

        Ok(stream_sse_chunks(
            upstream_response
                .bytes_stream()
                .map(|result| result.map(|bytes| bytes.to_vec()))
                .boxed(),
        ))
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
            max_tokens: request.max_tokens,
        };

        let response = self
            .with_api_request_headers(
                Self::http_client()
                    .post(self.chat_completion_url())
                    .json(&payload),
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

            return Err(AppError::provider(redact_for_logs(format!(
                "upstream provider returned status={status} body={body}"
            ))));
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

    let Some(first_choice) = response.choices.into_iter().next() else {
        return Err(AppError::provider(
            "upstream chat completion response returned no choices",
        ));
    };

    let content = first_choice
        .message
        .and_then(|message| message.content)
        .unwrap_or_default();

    Ok(ChatResponse {
        provider: provider_name,
        model: response.model.unwrap_or(model),
        content,
        usage: response.usage.as_ref().map(normalize_usage),
    })
}

#[cfg(test)]
fn parse_stream_events(raw_body: &str) -> AppResult<Vec<ChatStreamChunk>> {
    let mut chunks = Vec::new();
    let mut event_data = String::new();

    for raw_line in raw_body.lines() {
        let line = raw_line.trim_end();

        if line.is_empty() {
            if event_data.trim().is_empty() {
                continue;
            }

            chunks.extend(parse_stream_event(&event_data)?);
            event_data.clear();
            continue;
        }

        let trimmed = line.trim_start();
        if trimmed.starts_with(':') {
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("data:") {
            if !event_data.is_empty() {
                event_data.push('\n');
            }
            event_data.push_str(value.trim_start());
        }
    }

    if !event_data.trim().is_empty() {
        chunks.extend(parse_stream_event(&event_data)?);
    }

    if chunks.is_empty() {
        return Err(AppError::provider("upstream stream response was empty"));
    }

    Ok(chunks)
}

fn stream_sse_chunks(
    byte_stream: BoxStream<'static, Result<Vec<u8>, reqwest::Error>>,
) -> ChatCompletionStream {
    stream::unfold(SseStreamState::new(byte_stream), |mut state| async move {
        if state.finished {
            return None;
        }

        loop {
            if let Some(next) = state.pending.pop_front() {
                if next.is_ok() {
                    state.emitted_any = true;
                }
                return Some((next, state));
            }

            if let Some(position) = state.line_buffer.iter().position(|byte| *byte == b'\n') {
                let mut line = state.line_buffer.drain(..=position).collect::<Vec<_>>();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }

                if let Err(error) = process_sse_line(&mut state, &line) {
                    state.finished = true;
                    return Some((Err(error), state));
                }
                continue;
            }

            match state.byte_stream.next().await {
                Some(Ok(bytes)) => {
                    state.line_buffer.extend(bytes);
                }
                Some(Err(error)) => {
                    state.finished = true;
                    return Some((
                        Err(AppError::infrastructure(format!(
                            "upstream chat completion stream read failed: {error}"
                        ))),
                        state,
                    ));
                }
                None => {
                    if !state.line_buffer.is_empty() {
                        let line = std::mem::take(&mut state.line_buffer);
                        if let Err(error) = process_sse_line(&mut state, &line) {
                            state.finished = true;
                            return Some((Err(error), state));
                        }
                    }

                    if let Err(error) = flush_pending_sse_event(&mut state) {
                        state.finished = true;
                        return Some((Err(error), state));
                    }

                    if let Some(next) = state.pending.pop_front() {
                        if next.is_ok() {
                            state.emitted_any = true;
                        }
                        return Some((next, state));
                    }

                    state.finished = true;
                    if !state.emitted_any {
                        return Some((
                            Err(AppError::provider("upstream stream response was empty")),
                            state,
                        ));
                    }
                    return None;
                }
            }
        }
    })
    .boxed()
}

struct SseStreamState {
    byte_stream: BoxStream<'static, Result<Vec<u8>, reqwest::Error>>,
    line_buffer: Vec<u8>,
    event_data: String,
    pending: VecDeque<AppResult<ChatStreamChunk>>,
    emitted_any: bool,
    finished: bool,
}

impl SseStreamState {
    fn new(byte_stream: BoxStream<'static, Result<Vec<u8>, reqwest::Error>>) -> Self {
        Self {
            byte_stream,
            line_buffer: Vec::new(),
            event_data: String::new(),
            pending: VecDeque::new(),
            emitted_any: false,
            finished: false,
        }
    }
}

fn process_sse_line(state: &mut SseStreamState, raw_line: &[u8]) -> AppResult<()> {
    let line = std::str::from_utf8(raw_line)
        .map_err(|error| AppError::provider(format!("invalid upstream stream line: {error}")))?;

    if line.is_empty() {
        flush_pending_sse_event(state)?;
        return Ok(());
    }

    let trimmed = line.trim_start();
    if trimmed.starts_with(':') {
        return Ok(());
    }

    if let Some(value) = trimmed.strip_prefix("data:") {
        if !state.event_data.is_empty() {
            state.event_data.push('\n');
        }
        state.event_data.push_str(value.trim_start());
    }

    Ok(())
}

fn flush_pending_sse_event(state: &mut SseStreamState) -> AppResult<()> {
    if state.event_data.trim().is_empty() {
        state.event_data.clear();
        return Ok(());
    }

    let event_data = std::mem::take(&mut state.event_data);
    state
        .pending
        .extend(parse_stream_event(&event_data)?.into_iter().map(Ok));
    Ok(())
}

fn parse_stream_event(raw_event_data: &str) -> AppResult<Vec<ChatStreamChunk>> {
    let payload = raw_event_data.trim();
    if payload == "[DONE]" {
        return Ok(Vec::new());
    }

    let chunk: OpenAiChatCompletionChunk = serde_json::from_str(payload)
        .map_err(|error| AppError::provider(format!("invalid upstream stream payload: {error}")))?;

    let mut chunks = Vec::new();
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

    Ok(chunks)
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
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
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
    fn parse_non_stream_response_rejects_empty_choices() {
        let raw_response = r#"
        {
            "id": "chatcmpl-2",
            "object": "chat.completion",
            "model": "gpt-4o-mini",
            "choices": [],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 0,
                "total_tokens": 10,
                "estimated": false
            }
        }
        "#;

        assert!(
            parse_chat_completion_response(
                raw_response,
                "mizan/public-gpt".to_owned(),
                "openai".to_owned(),
            )
            .is_err()
        );
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

    #[test]
    fn parse_stream_events_joins_repeated_data_fields_with_newlines() {
        let raw_stream = "".to_string()
            + "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\n"
            + "data: \"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"finish_reason\":\"stop\"}]}\n"
            + "\n"
            + "data: [DONE]\n";

        let chunks = parse_stream_events(&raw_stream).expect("parse multi-line stream event");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta, "Hello");
        assert_eq!(chunks[0].finish_reason.as_deref(), Some("stop"));
    }

    #[tokio::test]
    async fn stream_sse_chunks_emits_before_source_finishes() {
        let first_event = "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n";
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        sender
            .send(Ok(first_event.as_bytes().to_vec()))
            .await
            .expect("send first stream bytes");

        let byte_stream = stream::unfold(receiver, |mut receiver| async {
            receiver.recv().await.map(|item| (item, receiver))
        })
        .boxed();
        let mut chunks = stream_sse_chunks(byte_stream);

        let first = tokio::time::timeout(std::time::Duration::from_millis(100), chunks.next())
            .await
            .expect("stream should emit before source closes")
            .expect("stream should yield a chunk")
            .expect("chunk should parse");

        assert_eq!(first.delta, "Hello");
    }
}
