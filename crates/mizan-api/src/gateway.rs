use std::convert::Infallible;

use axum::{
    Extension, Json,
    extract::State,
    http::{
        HeaderMap, StatusCode,
        header::{self, HeaderName, HeaderValue},
    },
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
};
use futures_util::{StreamExt, stream};
use mizan_core::{AppError, DatabaseBackend, ErrorEnvelope, RequestContext, RequestContextBuilder};
use mizan_providers::{
    ChatCompletionStream, ChatMessage, ChatRequest, ChatResponse, ChatStreamChunk,
    OpenAiCompatibleProvider,
};
use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, query_as};
use tracing::{info, warn};
use uuid::Uuid;

use crate::AppState;
use crate::auth::ApiKeyIdentity;
use crate::utils::{decrypt_provider_api_key, from_app_error, now_utc_epoch_seconds, prepare_sql};

type GatewayHttpResult = Result<Response, (StatusCode, Json<ErrorEnvelope>)>;

#[derive(Debug, Deserialize)]
pub struct ChatCompletionsRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionsMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionsChoice {
    pub index: usize,
    pub message: ChatCompletionsMessage,
    pub finish_reason: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionsUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionsResponse {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatCompletionsChoice>,
    pub usage: Option<ChatCompletionsUsage>,
}

#[derive(Debug, Serialize)]
struct ChatCompletionsStreamChoice {
    pub index: usize,
    pub delta: ChatCompletionsMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChatCompletionsStreamResponse {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatCompletionsStreamChoice>,
}

#[derive(Debug)]
struct ResolvedModelRoute {
    id: Uuid,
    provider_connection_id: Uuid,
    upstream_model: String,
    provider_type: String,
    provider_base_url: String,
    provider_api_key: String,
}

pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
    headers: HeaderMap,
    Json(payload): Json<ChatCompletionsRequest>,
) -> GatewayHttpResult {
    let request_id = parse_request_id_header(&headers, "x-request-id").unwrap_or_else(Uuid::now_v7);
    let trace_id = parse_request_id_header(&headers, "x-trace-id").unwrap_or(request_id);

    let public_model = payload.model.trim();
    let mut context = RequestContextBuilder::default()
        .user_id(identity.user_id)
        .api_key_id(identity.api_key_id)
        .request_id(request_id)
        .trace_id(trace_id)
        .streaming(payload.stream)
        .build();

    if public_model.is_empty() {
        return Ok(build_error_response(
            &context,
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "chat_completion.model",
                "model is required",
            ))),
        ));
    }

    let route = match resolve_model_route(
        &state.database,
        state.database_backend(),
        state.config.provider_secret_key.as_deref(),
        public_model,
    )
    .await
    {
        Ok(route) => route,
        Err(error) => {
            let (status, body) = from_app_error(error);
            return Ok(build_error_response(&context, status, body));
        }
    };

    context = RequestContextBuilder::default()
        .user_id(identity.user_id)
        .api_key_id(identity.api_key_id)
        .provider(route.provider_type.clone())
        .request_id(request_id)
        .trace_id(trace_id)
        .route(public_model.to_string())
        .route_id(route.id)
        .provider_id(route.provider_connection_id)
        .model(route.upstream_model.clone())
        .streaming(payload.stream)
        .build();

    info!(
        request_id = %context.request_id,
        trace_id = %context.trace_id,
        user_id = %context.user_id.map_or("unknown".to_owned(), |value| value.to_string()),
        api_key_id = %context.api_key_id.map_or("unknown".to_owned(), |value| value.to_string()),
        route = %context.route.clone().unwrap_or_default(),
        streaming = context.streaming,
        "chat completion request",
    );

    let upstream_request = ChatRequest {
        model: route.upstream_model.clone(),
        messages: payload.messages.clone(),
        stream: payload.stream,
    };

    let completion_id = format!("chatcmpl-{}", Uuid::now_v7());

    let provider_name = if route.provider_type.eq_ignore_ascii_case("openai") {
        "openai".to_owned()
    } else {
        "openai-compatible".to_owned()
    };
    let provider = OpenAiCompatibleProvider::new(
        provider_name,
        route.provider_base_url,
        route.provider_api_key,
    );

    let response = if payload.stream {
        let upstream = match state
            .gateway
            .chat_completions_stream(&context, &provider, upstream_request)
            .await
        {
            Ok(upstream) => upstream,
            Err(error) => {
                let (status, body) = from_app_error(normalize_provider_error(
                    error,
                    &context,
                    public_model.to_string(),
                ));
                return Ok(build_error_response(&context, status, body));
            }
        };

        stream_chat_completion_response(
            &completion_id,
            public_model.to_string(),
            upstream,
            &context,
        )
    } else {
        let upstream_response = match state
            .gateway
            .chat_completions(&context, &provider, upstream_request)
            .await
        {
            Ok(upstream_response) => upstream_response,
            Err(error) => {
                let (status, body) = from_app_error(normalize_provider_error(
                    error,
                    &context,
                    public_model.to_string(),
                ));
                return Ok(build_error_response(&context, status, body));
            }
        };

        json_chat_completion_response(
            &completion_id,
            public_model.to_string(),
            upstream_response,
            &context,
        )
    };

    Ok(response)
}

fn parse_request_id_header(headers: &HeaderMap, header_name: &str) -> Option<Uuid> {
    match headers.get(header_name) {
        None => None,
        Some(value) => {
            let raw_value = match value.to_str() {
                Ok(value) => value,
                Err(_) => {
                    warn!(header_name, "invalid request id header bytes, ignoring");
                    return None;
                }
            };

            match Uuid::parse_str(raw_value) {
                Ok(value) => Some(value),
                Err(_) => {
                    warn!(
                        header_name,
                        value = raw_value,
                        "invalid request ID/trace ID header value, ignoring"
                    );
                    None
                }
            }
        }
    }
}

fn build_error_response(
    context: &RequestContext,
    status: StatusCode,
    body: Json<ErrorEnvelope>,
) -> Response {
    let mut response = (status, body).into_response();
    attach_request_headers(&mut response, context);
    response
}

fn normalize_provider_error(
    error: AppError,
    context: &RequestContext,
    route_alias: String,
) -> AppError {
    let provider = context.provider.as_deref().unwrap_or("unknown");
    let request_id = context.request_id;
    match error {
        AppError::Infrastructure(message) => AppError::Provider(format!(
            "upstream transport failure route={route_alias} provider={provider} request_id={request_id}: {message}"
        )),
        AppError::Provider(message) => AppError::Provider(format!(
            "upstream provider error route={route_alias} provider={provider} request_id={request_id}: {message}"
        )),
        other => other,
    }
}

fn json_chat_completion_response(
    completion_id: &str,
    model: String,
    upstream: ChatResponse,
    context: &RequestContext,
) -> Response {
    let payload = map_to_chat_completion_response(completion_id.to_string(), model, upstream);
    let mut response = Json(payload).into_response();
    attach_request_headers(&mut response, context);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn stream_chat_completion_response(
    completion_id: &str,
    model: String,
    upstream: ChatCompletionStream,
    context: &RequestContext,
) -> Response {
    let events = build_stream_events(completion_id, model, upstream, context);
    let mut response = Sse::new(events).into_response();
    attach_request_headers(&mut response, context);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
}

fn build_stream_events(
    completion_id: &str,
    model: String,
    upstream: ChatCompletionStream,
    context: &RequestContext,
) -> impl futures_util::Stream<Item = Result<Event, Infallible>> + Send + 'static {
    let created = now_utc_epoch_seconds();
    let completion_id = completion_id.to_string();
    let context = context.clone();
    let route_alias = model.clone();

    struct StreamBuildState {
        upstream: ChatCompletionStream,
        completion_id: String,
        model: String,
        route_alias: String,
        context: RequestContext,
        created: i64,
        emit_done: bool,
    }

    stream::unfold(
        StreamBuildState {
            upstream,
            completion_id,
            model,
            route_alias,
            context,
            created,
            emit_done: true,
        },
        |mut state| async move {
            if !state.emit_done {
                return None;
            }

            match state.upstream.next().await {
                Some(upstream_chunk) => {
                    let event = match upstream_chunk {
                        Ok(upstream_chunk) => {
                            let chunk = map_to_chat_completion_stream_response(
                                state.completion_id.clone(),
                                state.model.clone(),
                                state.created,
                                upstream_chunk,
                            );
                            Event::default()
                                .json_data(chunk)
                                .expect("chat completion chunk should serialize")
                        }
                        Err(error) => {
                            state.emit_done = false;
                            let error = normalize_provider_error(
                                error,
                                &state.context,
                                state.route_alias.clone(),
                            );
                            Event::default()
                                .event("error")
                                .json_data(ErrorEnvelope::from(&error))
                                .expect("error envelope should serialize")
                        }
                    };
                    Some((Ok(event), state))
                }
                None => {
                    if state.emit_done {
                        state.emit_done = false;
                        Some((Ok(Event::default().data("[DONE]")), state))
                    } else {
                        None
                    }
                }
            }
        },
    )
}

fn map_to_chat_completion_response(
    completion_id: String,
    model: String,
    upstream: ChatResponse,
) -> ChatCompletionsResponse {
    ChatCompletionsResponse {
        id: completion_id,
        object: "chat.completion",
        created: now_utc_epoch_seconds(),
        model,
        choices: vec![ChatCompletionsChoice {
            index: 0,
            message: ChatCompletionsMessage {
                role: "assistant".to_string(),
                content: upstream.content,
            },
            finish_reason: "stop",
        }],
        usage: upstream.usage.map(|usage| ChatCompletionsUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        }),
    }
}

fn map_to_chat_completion_stream_response(
    completion_id: String,
    model: String,
    created: i64,
    upstream: ChatStreamChunk,
) -> ChatCompletionsStreamResponse {
    ChatCompletionsStreamResponse {
        id: completion_id,
        object: "chat.completion.chunk",
        created,
        model,
        choices: vec![ChatCompletionsStreamChoice {
            index: upstream.index,
            delta: ChatCompletionsMessage {
                role: "assistant".to_string(),
                content: upstream.delta,
            },
            finish_reason: upstream.finish_reason,
        }],
    }
}

fn attach_request_headers(response: &mut Response, context: &RequestContext) {
    response.headers_mut().insert(
        HeaderName::from_static("x-request-id"),
        HeaderValue::from_str(&context.request_id.to_string())
            .expect("request id must be valid uuid"),
    );
    response.headers_mut().insert(
        header::HeaderName::from_static("x-trace-id"),
        HeaderValue::from_str(&context.trace_id.to_string()).expect("trace id must be valid uuid"),
    );
}

async fn resolve_model_route(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    provider_secret_key: Option<&str>,
    public_model: &str,
) -> Result<ResolvedModelRoute, AppError> {
    let resolved = query_as::<_, (String, String, String, String, String, String)>(&prepare_sql(
        database_backend,
        "SELECT mr.id,
                mr.upstream_model,
                pc.provider_type,
                pc.id,
                pc.base_url,
                pc.api_key_encrypted
         FROM model_routes mr
         INNER JOIN provider_connections pc
            ON pc.id = mr.provider_connection_id
         WHERE mr.public_model = ? AND mr.enabled = ? AND pc.enabled = ?",
    ))
    .bind(public_model)
    .bind(1)
    .bind(1)
    .fetch_optional(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?
    .ok_or_else(|| {
        AppError::invalid_config("chat_completion.model", "model not found or disabled")
    })?;

    let (
        route_id,
        upstream_model,
        provider_type,
        provider_connection_id,
        provider_base_url,
        encrypted_api_key,
    ) = resolved;
    let id = Uuid::parse_str(&route_id).map_err(|error| {
        AppError::infrastructure(format!("stored route id is invalid: {error}"))
    })?;
    let provider_connection_id = Uuid::parse_str(&provider_connection_id).map_err(|error| {
        AppError::infrastructure(format!(
            "stored provider connection id for route is invalid: {error}"
        ))
    })?;
    let provider_secret_key = provider_secret_key.ok_or_else(|| {
        AppError::invalid_config(
            "MIZAN_PROVIDER_SECRET_KEY",
            "set MIZAN_PROVIDER_SECRET_KEY before resolving model routes",
        )
    })?;
    let provider_api_key = decrypt_provider_api_key(
        provider_secret_key,
        &provider_connection_id.to_string(),
        &encrypted_api_key,
    )?;

    Ok(ResolvedModelRoute {
        id,
        provider_connection_id,
        upstream_model,
        provider_type: provider_type.trim().to_string(),
        provider_base_url,
        provider_api_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_to_chat_completion_response_uses_model_and_content() {
        let upstream_model = "openai/gpt-4o-mini".to_string();
        let alias = "mizan-public-gpt-4o-mini".to_string();
        let upstream = ChatResponse {
            provider: "openai".to_string(),
            model: upstream_model.clone(),
            content: "pong".to_string(),
            usage: Some(mizan_providers::TokenUsage {
                prompt_tokens: 7,
                completion_tokens: 3,
                total_tokens: 10,
                estimated: false,
            }),
        };
        let completion_id = format!("chatcmpl-{}", Uuid::now_v7());

        let response = map_to_chat_completion_response(completion_id, alias.clone(), upstream);
        assert_eq!(response.choices.len(), 1);
        assert_eq!(response.choices[0].message.content, "pong");
        assert_eq!(response.model, alias);
    }

    #[tokio::test]
    async fn build_stream_events_emits_before_upstream_finishes() {
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        sender
            .send(Ok(ChatStreamChunk {
                index: 0,
                delta: "hello".to_string(),
                finish_reason: None,
                usage: None,
            }))
            .await
            .expect("send first upstream chunk");

        let upstream = stream::unfold(receiver, |mut receiver| async {
            receiver.recv().await.map(|item| (item, receiver))
        })
        .boxed();
        let context = RequestContext::new();
        let events = build_stream_events(
            "chatcmpl-test",
            "mizan-public-model".to_string(),
            upstream,
            &context,
        );
        futures_util::pin_mut!(events);

        let first = tokio::time::timeout(std::time::Duration::from_millis(100), events.next())
            .await
            .expect("gateway should emit before upstream closes")
            .expect("gateway stream should yield an event")
            .expect("event should be infallible");

        assert!(format!("{first:?}").contains("chat.completion.chunk"));
    }
}
