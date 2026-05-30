use std::convert::Infallible;
use std::time::Instant;

use crate::logging::{RequestLogInput, error_code_from_app_error, record_request_log};
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
use mizan_core::{
    AppError, DatabaseBackend, ErrorEnvelope, RequestContext, RequestContextBuilder,
    redact_for_logs,
};
use mizan_limits::{LimitScope, RuntimeLimitLease, RuntimeLimitPolicy, RuntimeLimitRequest};
use mizan_providers::{
    ChatCompletionStream, ChatMessage, ChatRequest, ChatResponse, ChatStreamChunk,
    OpenAiCompatibleProvider,
};
use mizan_wallet::RoutePrice;
use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, FromRow, query_as};
use tokio::task;
use tracing::{Instrument, info, info_span, warn};
use uuid::Uuid;

use crate::AppState;
use crate::auth::ApiKeyIdentity;
use crate::billing;
use crate::metrics::{GatewayObservation, MetricsRegistry};
use crate::utils::{decrypt_provider_api_key, from_app_error, now_utc_epoch_seconds, prepare_sql};

type GatewayHttpResult = Result<Response, (StatusCode, Json<ErrorEnvelope>)>;

const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const RESPONSES_PATH: &str = "/v1/responses";
const CHAT_COMPLETIONS_MODEL_FIELD: &str = "chat_completion.model";
const CHAT_COMPLETIONS_STREAM_FIELD: &str = "chat_completion.stream";
const CHAT_COMPLETIONS_MAX_TOKENS_FIELD: &str = "chat_completion.max_tokens";
const RESPONSES_MODEL_FIELD: &str = "responses.model";
const RESPONSES_STREAM_FIELD: &str = "responses.stream";
const RESPONSES_MAX_TOKENS_FIELD: &str = "responses.max_tokens";

#[derive(Clone, Copy, Debug)]
struct GatewayRequestSpec {
    path: &'static str,
    kind: &'static str,
    model_field: &'static str,
    stream_field: &'static str,
    max_tokens_field: &'static str,
    allow_stream: bool,
}

const CHAT_COMPLETIONS_SPEC: GatewayRequestSpec = GatewayRequestSpec {
    path: CHAT_COMPLETIONS_PATH,
    kind: "chat_completion",
    model_field: CHAT_COMPLETIONS_MODEL_FIELD,
    stream_field: CHAT_COMPLETIONS_STREAM_FIELD,
    max_tokens_field: CHAT_COMPLETIONS_MAX_TOKENS_FIELD,
    allow_stream: true,
};

const RESPONSES_SPEC: GatewayRequestSpec = GatewayRequestSpec {
    path: RESPONSES_PATH,
    kind: "responses",
    model_field: RESPONSES_MODEL_FIELD,
    stream_field: RESPONSES_STREAM_FIELD,
    max_tokens_field: RESPONSES_MAX_TOKENS_FIELD,
    allow_stream: false,
};

#[derive(Debug, Deserialize)]
pub struct ChatCompletionsRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub max_tokens: Option<u64>,
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
    max_tokens: Option<u64>,
    input_price_per_1m_tokens: i64,
    output_price_per_1m_tokens: i64,
}

impl ResolvedModelRoute {
    fn route_price(&self) -> RoutePrice {
        RoutePrice {
            input_microcredits_per_1m_tokens: self.input_price_per_1m_tokens,
            output_microcredits_per_1m_tokens: self.output_price_per_1m_tokens,
        }
    }
}

pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
    headers: HeaderMap,
    Json(payload): Json<ChatCompletionsRequest>,
) -> GatewayHttpResult {
    chat_completions_impl(state, identity, headers, payload, CHAT_COMPLETIONS_SPEC).await
}

pub async fn responses(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
    headers: HeaderMap,
    Json(payload): Json<ChatCompletionsRequest>,
) -> GatewayHttpResult {
    chat_completions_impl(state, identity, headers, payload, RESPONSES_SPEC).await
}

async fn chat_completions_impl(
    state: AppState,
    identity: ApiKeyIdentity,
    headers: HeaderMap,
    payload: ChatCompletionsRequest,
    spec: GatewayRequestSpec,
) -> GatewayHttpResult {
    let request_id = parse_request_id_header(&headers, "x-request-id").unwrap_or_else(Uuid::now_v7);
    let trace_id = parse_request_id_header(&headers, "x-trace-id").unwrap_or(request_id);
    let request_started_at = Instant::now();

    let public_model = payload.model.trim();
    let mut context = RequestContextBuilder::default()
        .user_id(identity.user_id)
        .api_key_id(identity.api_key_id)
        .request_id(request_id)
        .trace_id(trace_id)
        .method("POST")
        .path(spec.path)
        .streaming(payload.stream)
        .build();

    if public_model.is_empty() {
        let app_error = AppError::invalid_config(spec.model_field, "model is required");
        let status = app_error_status_code(&app_error);
        let error_code = error_code_from_app_error(&app_error);
        record_gateway_request_completion(
            &state.database,
            state.database_backend(),
            &context,
            &request_started_at,
            status,
            Some(public_model),
            None,
            Some(&error_code),
        )
        .await;
        return Ok(build_error_response(
            &context,
            status,
            Json(ErrorEnvelope::from(&app_error)),
        ));
    }

    if !spec.allow_stream && payload.stream {
        let app_error = AppError::invalid_config(
            spec.stream_field,
            "stream is not supported for this endpoint yet",
        );
        let status = app_error_status_code(&app_error);
        let error_code = error_code_from_app_error(&app_error);
        record_gateway_request_completion(
            &state.database,
            state.database_backend(),
            &context,
            &request_started_at,
            status,
            Some(public_model),
            None,
            Some(&error_code),
        )
        .await;
        return Ok(build_error_response(
            &context,
            status,
            Json(ErrorEnvelope::from(&app_error)),
        ));
    }

    let route = match resolve_model_route(
        &state.database,
        state.database_backend(),
        state.config.provider_secret_key.as_deref(),
        spec.model_field,
        public_model,
    )
    .instrument(info_span!(
        "route_resolution",
        request_id = %request_id,
        trace_id = %trace_id,
        route = %public_model,
    ))
    .await
    {
        Ok(route) => route,
        Err(error) => {
            let status = app_error_status_code(&error);
            let error_code = error_code_from_app_error(&error);
            record_gateway_request_completion(
                &state.database,
                state.database_backend(),
                &context,
                &request_started_at,
                status,
                Some(public_model),
                None,
                Some(&error_code),
            )
            .await;
            let (_, body) = from_app_error(error);
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
        .method("POST")
        .path(spec.path)
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
        request_kind = spec.kind,
        request_path = spec.path,
        "gateway request",
    );

    let effective_max_tokens = match resolve_effective_max_tokens(
        payload.max_tokens,
        route.max_tokens,
        spec.max_tokens_field,
    ) {
        Ok(max_tokens) => max_tokens,
        Err(error) => {
            let status = app_error_status_code(&error);
            let error_code = error_code_from_app_error(&error);
            record_gateway_request_completion(
                &state.database,
                state.database_backend(),
                &context,
                &request_started_at,
                status,
                Some(public_model),
                None,
                Some(&error_code),
            )
            .await;
            let (_, body) = from_app_error(error);
            return Ok(build_error_response(&context, status, body));
        }
    };

    let upstream_request = ChatRequest {
        model: route.upstream_model.clone(),
        messages: payload.messages.clone(),
        stream: payload.stream,
        max_tokens: effective_max_tokens,
    };
    let request_messages = upstream_request.messages.clone();

    let completion_id = format!("chatcmpl-{}", Uuid::now_v7());

    let provider_name = if route.provider_type.eq_ignore_ascii_case("openai") {
        "openai".to_owned()
    } else {
        "openai-compatible".to_owned()
    };
    let route_price = route.route_price();
    let provider = OpenAiCompatibleProvider::new(
        provider_name.clone(),
        route.provider_base_url,
        route.provider_api_key.clone(),
    );
    let admission_usage = estimate_admission_usage(&request_messages, effective_max_tokens);
    let prompt_only_usage = billing::estimate_usage(&request_messages, "");
    if let Err(error) = billing::ensure_sufficient_credit(
        &state.database,
        state.database_backend(),
        identity.user_id,
        admission_usage,
        route_price,
    )
    .instrument(info_span!(
        "credit_preflight",
        request_id = %context.request_id,
        trace_id = %context.trace_id,
        route = %public_model,
    ))
    .await
    {
        warn!(
            request_id = %request_id,
            error = %error,
            "credit preflight failed"
        );
        let status = app_error_status_code(&error);
        let error_code = error_code_from_app_error(&error);
        let latency_ms = request_started_at.elapsed().as_millis() as u64;
        record_gateway_request_completion(
            &state.database,
            state.database_backend(),
            &context,
            &request_started_at,
            status,
            Some(public_model),
            Some(&provider_name),
            Some(&error_code),
        )
        .await;
        let (status, body) = from_app_error(error);
        if let Err(error) = billing::record_usage(
            &state.database,
            state.database_backend(),
            billing::BillingInput {
                request_id,
                user_id: identity.user_id,
                api_key_id: Some(identity.api_key_id),
                provider_id: Some(route.provider_connection_id),
                route_id: Some(route.id),
                model: public_model.to_string(),
                usage: prompt_only_usage,
                status_code: status.as_u16(),
                latency_ms,
                route_price,
            },
        )
        .await
        {
            warn!(
                request_id = %request_id,
                error = %error,
                "failed to persist insufficient-credit preflight usage"
            );
        }
        observe_gateway_metrics(
            &state.metrics,
            &context,
            public_model,
            prompt_only_usage,
            status,
            request_started_at.elapsed().as_millis() as u64,
            route_price,
        );
        return Ok(build_error_response(&context, status, body));
    }

    let estimated_total_tokens = admission_usage.total_tokens;
    let limit_lease = match acquire_runtime_limits(
        &state,
        vec![
            LimitScope::ApiKey(identity.api_key_id),
            LimitScope::User(identity.user_id),
            LimitScope::Provider(route.provider_connection_id),
        ],
        estimated_total_tokens,
    )
    .instrument(info_span!(
        "runtime_limits",
        request_id = %context.request_id,
        trace_id = %context.trace_id,
        route = %public_model,
    ))
    .await
    {
        Ok(lease) => lease,
        Err(error) => {
            let status = app_error_status_code(&error);
            let error_code = error_code_from_app_error(&error);
            let latency_ms = request_started_at.elapsed().as_millis() as u64;
            record_gateway_request_completion(
                &state.database,
                state.database_backend(),
                &context,
                &request_started_at,
                status,
                Some(public_model),
                Some(&provider_name),
                Some(&error_code),
            )
            .await;
            let (_, body) = from_app_error(error);
            observe_gateway_metrics(
                &state.metrics,
                &context,
                public_model,
                admission_usage,
                status,
                latency_ms,
                route_price,
            );
            return Ok(build_error_response(&context, status, body));
        }
    };

    let response = if payload.stream {
        let billing_context = StreamBillingContext {
            request_started_at,
            database: state.database.clone(),
            database_backend: state.database_backend(),
            route_price,
            user_id: identity.user_id,
            api_key_id: Some(identity.api_key_id),
            provider_id: route.provider_connection_id,
            route_id: route.id,
            limit_lease: Some(limit_lease),
            metrics: state.metrics.clone(),
        };

        let upstream = match state
            .gateway
            .chat_completions_stream(&context, &provider, upstream_request)
            .instrument(info_span!(
                "provider_stream_call",
                request_id = %context.request_id,
                trace_id = %context.trace_id,
                route = %public_model,
                provider = %context.provider.clone().unwrap_or_default(),
                model = %context.model.clone().unwrap_or_default(),
            ))
            .await
        {
            Ok(upstream) => upstream,
            Err(error) => {
                let normalized_error =
                    normalize_provider_error(error, &context, public_model.to_string());
                let error_code = error_code_from_app_error(&normalized_error);
                let latency_ms = request_started_at.elapsed().as_millis() as u64;
                let (status, body) = from_app_error(normalized_error);
                let usage = billing::estimate_usage(&request_messages, "");
                if let Err(error) = billing::record_usage(
                    &state.database,
                    state.database_backend(),
                    billing::BillingInput {
                        request_id,
                        user_id: identity.user_id,
                        api_key_id: Some(identity.api_key_id),
                        provider_id: Some(route.provider_connection_id),
                        route_id: Some(route.id),
                        model: public_model.to_string(),
                        usage,
                        status_code: status.as_u16(),
                        latency_ms,
                        route_price,
                    },
                )
                .instrument(info_span!(
                    "metering",
                    request_id = %context.request_id,
                    trace_id = %context.trace_id,
                    route = %public_model,
                    status = status.as_u16(),
                ))
                .await
                {
                    warn!(
                        request_id = %request_id,
                        error = %error,
                        "failed to persist stream request usage for upstream stream init error"
                    );
                }
                observe_gateway_metrics(
                    &state.metrics,
                    &context,
                    public_model,
                    usage,
                    status,
                    latency_ms,
                    route_price,
                );
                record_gateway_request_completion(
                    &state.database,
                    state.database_backend(),
                    &context,
                    &request_started_at,
                    status,
                    Some(public_model),
                    Some(&provider_name),
                    Some(error_code.as_str()),
                )
                .await;
                release_limit_lease(billing_context.limit_lease);
                return Ok(build_error_response(&context, status, body));
            }
        };

        stream_chat_completion_response(
            &completion_id,
            public_model.to_string(),
            upstream,
            request_messages.clone(),
            &context,
            billing_context,
        )
    } else {
        let upstream_response = match state
            .gateway
            .chat_completions(&context, &provider, upstream_request)
            .instrument(info_span!(
                "provider_call",
                request_id = %context.request_id,
                trace_id = %context.trace_id,
                route = %public_model,
                provider = %context.provider.clone().unwrap_or_default(),
                model = %context.model.clone().unwrap_or_default(),
            ))
            .await
        {
            Ok(upstream_response) => upstream_response,
            Err(error) => {
                let normalized_error =
                    normalize_provider_error(error, &context, public_model.to_string());
                let error_code = error_code_from_app_error(&normalized_error);
                let (status, body) = from_app_error(normalized_error);
                let latency_ms = request_started_at.elapsed().as_millis() as u64;
                let usage = billing::estimate_usage(&request_messages, "");
                if let Err(error) = billing::record_usage(
                    &state.database,
                    state.database_backend(),
                    billing::BillingInput {
                        request_id,
                        user_id: identity.user_id,
                        api_key_id: Some(identity.api_key_id),
                        provider_id: Some(route.provider_connection_id),
                        route_id: Some(route.id),
                        model: public_model.to_string(),
                        usage,
                        status_code: status.as_u16(),
                        latency_ms,
                        route_price,
                    },
                )
                .instrument(info_span!(
                    "metering",
                    request_id = %context.request_id,
                    trace_id = %context.trace_id,
                    route = %public_model,
                    status = status.as_u16(),
                ))
                .await
                {
                    warn!(
                        request_id = %request_id,
                        error = %error,
                        "failed to persist non-stream request usage for upstream error"
                    );
                }
                observe_gateway_metrics(
                    &state.metrics,
                    &context,
                    public_model,
                    usage,
                    status,
                    latency_ms,
                    route_price,
                );
                record_gateway_request_completion(
                    &state.database,
                    state.database_backend(),
                    &context,
                    &request_started_at,
                    status,
                    Some(public_model),
                    Some(&provider_name),
                    Some(error_code.as_str()),
                )
                .await;
                release_limit_lease(Some(limit_lease));
                return Ok(build_error_response(&context, status, body));
            }
        };

        let usage = upstream_response.usage.unwrap_or_else(|| {
            billing::estimate_usage(&request_messages, &upstream_response.content)
        });
        let latency_ms = request_started_at.elapsed().as_millis() as u64;

        if let Err(error) = billing::record_usage(
            &state.database,
            state.database_backend(),
            billing::BillingInput {
                request_id,
                user_id: identity.user_id,
                api_key_id: Some(identity.api_key_id),
                provider_id: Some(route.provider_connection_id),
                route_id: Some(route.id),
                model: public_model.to_string(),
                usage,
                status_code: StatusCode::OK.as_u16(),
                latency_ms,
                route_price,
            },
        )
        .instrument(info_span!(
            "metering",
            request_id = %context.request_id,
            trace_id = %context.trace_id,
            route = %public_model,
            status = StatusCode::OK.as_u16(),
        ))
        .await
        {
            let status = app_error_status_code(&error);
            let error_code = error_code_from_app_error(&error);
            let (_, body) = from_app_error(error);
            observe_gateway_metrics(
                &state.metrics,
                &context,
                public_model,
                usage,
                status,
                latency_ms,
                route_price,
            );
            record_gateway_request_completion(
                &state.database,
                state.database_backend(),
                &context,
                &request_started_at,
                status,
                Some(public_model),
                Some(&provider_name),
                Some(error_code.as_str()),
            )
            .await;
            release_limit_lease(Some(limit_lease));
            return Ok(build_error_response(&context, status, body));
        }

        observe_gateway_metrics(
            &state.metrics,
            &context,
            public_model,
            usage,
            StatusCode::OK,
            latency_ms,
            route_price,
        );
        record_gateway_request_completion(
            &state.database,
            state.database_backend(),
            &context,
            &request_started_at,
            StatusCode::OK,
            Some(public_model),
            Some(&provider_name),
            None,
        )
        .await;
        let response = json_chat_completion_response(
            &completion_id,
            public_model.to_string(),
            upstream_response,
            &context,
        );
        release_limit_lease(Some(limit_lease));
        response
    };

    Ok(response)
}

#[allow(clippy::too_many_arguments)]
async fn record_gateway_request_completion(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    context: &RequestContext,
    request_started_at: &Instant,
    status: StatusCode,
    route_alias: Option<&str>,
    provider_alias: Option<&str>,
    error_code: Option<&str>,
) {
    let route = context
        .route
        .clone()
        .or_else(|| route_alias.map(|value| value.to_string()));
    let provider = context
        .provider
        .clone()
        .or_else(|| provider_alias.map(|value| value.to_string()));
    let latency_ms = request_started_at.elapsed().as_millis() as u64;

    let database = database.clone();
    let request_log = RequestLogInput {
        request_id: context.request_id,
        user_id: context.user_id,
        api_key_id: context.api_key_id,
        provider_id: context.provider_id,
        route_id: context.route_id,
        method: context.method.clone().unwrap_or_else(|| "POST".to_owned()),
        path: context
            .path
            .clone()
            .unwrap_or_else(|| "/v1/chat/completions".to_owned()),
        route,
        provider,
        status_code: status,
        latency_ms,
        error_code: error_code.map(|value| value.to_string()),
    };

    let _request_log_task = task::spawn(async move {
        if let Err(error) = record_request_log(&database, database_backend, &request_log).await {
            warn!(error = %error, "failed to persist gateway request log");
        }
    });
}

async fn acquire_runtime_limits(
    state: &AppState,
    scopes: Vec<LimitScope>,
    estimated_prompt_tokens: u64,
) -> Result<RuntimeLimitLease, AppError> {
    let policy = RuntimeLimitPolicy {
        requests_per_window: state.config.limit_rpm,
        tokens_per_window: state.config.limit_tpm,
        concurrent_requests: state.config.limit_concurrency,
        window_seconds: state.config.limit_window_seconds,
        lease_seconds: state.config.limit_lease_seconds,
    };
    let client = state.redis.clone();
    task::spawn_blocking(move || {
        mizan_limits::check_and_acquire(
            client,
            policy,
            RuntimeLimitRequest {
                scopes,
                estimated_prompt_tokens,
            },
        )
    })
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?
}

fn release_limit_lease(lease: Option<RuntimeLimitLease>) {
    let Some(lease) = lease else {
        return;
    };

    let _release_task = task::spawn_blocking(move || {
        if let Err(error) = lease.release() {
            warn!(error = %error, "failed to release runtime limit lease");
        }
    });
}

fn observe_gateway_metrics(
    metrics: &MetricsRegistry,
    context: &RequestContext,
    route_alias: &str,
    usage: mizan_providers::TokenUsage,
    status: StatusCode,
    latency_ms: u64,
    route_price: RoutePrice,
) {
    metrics.observe_gateway(GatewayObservation {
        route: context
            .route
            .clone()
            .unwrap_or_else(|| route_alias.to_string()),
        provider: context
            .provider
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        model: context
            .model
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        status: status.as_u16(),
        usage,
        latency_ms,
        route_price,
    });
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

fn resolve_effective_max_tokens(
    requested_max_tokens: Option<u64>,
    route_max_tokens: Option<u64>,
    field_name: &'static str,
) -> Result<Option<u64>, AppError> {
    match (requested_max_tokens, route_max_tokens) {
        (Some(requested), Some(route_limit)) if requested > route_limit => Err(
            AppError::invalid_config(field_name, "max_tokens exceeds route limit"),
        ),
        (Some(requested), _) => Ok(Some(requested)),
        (None, route_limit) => Ok(route_limit),
    }
}

fn estimate_admission_usage(
    request_messages: &[ChatMessage],
    expected_completion_tokens: Option<u64>,
) -> mizan_providers::TokenUsage {
    let prompt_tokens = billing::estimate_usage(request_messages, "").prompt_tokens;
    let completion_tokens = expected_completion_tokens.unwrap_or_default();

    mizan_providers::TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        estimated: true,
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
        AppError::Infrastructure(message) => AppError::Provider(redact_for_logs(format!(
            "upstream transport failure route={route_alias} provider={provider} request_id={request_id}: {message}"
        ))),
        AppError::Provider(message) => AppError::Provider(redact_for_logs(format!(
            "upstream provider error route={route_alias} provider={provider} request_id={request_id}: {message}"
        ))),
        other => other,
    }
}

fn app_error_status_code(error: &AppError) -> StatusCode {
    match error {
        AppError::InvalidConfig { .. } => StatusCode::BAD_REQUEST,
        AppError::NotFound(_) => StatusCode::NOT_FOUND,
        AppError::Unauthorized => StatusCode::UNAUTHORIZED,
        AppError::Forbidden => StatusCode::FORBIDDEN,
        AppError::Provider(_) => StatusCode::BAD_GATEWAY,
        AppError::LimitExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
        AppError::InsufficientCredit => StatusCode::PAYMENT_REQUIRED,
        AppError::Config { .. } | AppError::Infrastructure(_) | AppError::Io(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
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
    request_messages: Vec<ChatMessage>,
    context: &RequestContext,
    billing_context: StreamBillingContext,
) -> Response {
    let events = build_stream_events(
        completion_id,
        model,
        upstream,
        request_messages,
        context,
        billing_context,
    );
    let mut response = Sse::new(events).into_response();
    attach_request_headers(&mut response, context);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
}

#[derive(Debug)]
struct StreamBillingContext {
    request_started_at: Instant,
    database: AnyPool,
    database_backend: DatabaseBackend,
    route_price: RoutePrice,
    user_id: Uuid,
    api_key_id: Option<Uuid>,
    provider_id: Uuid,
    route_id: Uuid,
    limit_lease: Option<RuntimeLimitLease>,
    metrics: MetricsRegistry,
}

fn build_stream_events(
    completion_id: &str,
    model: String,
    upstream: ChatCompletionStream,
    request_messages: Vec<ChatMessage>,
    context: &RequestContext,
    billing_context: StreamBillingContext,
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
        request_messages: Vec<ChatMessage>,
        latest_usage: Option<mizan_providers::TokenUsage>,
        created: i64,
        request_started_at: Instant,
        route_price: RoutePrice,
        user_id: Uuid,
        api_key_id: Option<Uuid>,
        provider_id: Uuid,
        route_id: Uuid,
        database: AnyPool,
        database_backend: DatabaseBackend,
        emit_done: bool,
        limit_lease: Option<RuntimeLimitLease>,
        metrics: MetricsRegistry,
    }

    impl Drop for StreamBuildState {
        fn drop(&mut self) {
            release_limit_lease(self.limit_lease.take());
        }
    }

    stream::unfold(
        StreamBuildState {
            upstream,
            completion_id,
            model,
            route_alias,
            context,
            request_messages,
            latest_usage: None,
            created,
            request_started_at: billing_context.request_started_at,
            route_price: billing_context.route_price,
            user_id: billing_context.user_id,
            api_key_id: billing_context.api_key_id,
            provider_id: billing_context.provider_id,
            route_id: billing_context.route_id,
            database: billing_context.database,
            database_backend: billing_context.database_backend,
            emit_done: true,
            limit_lease: billing_context.limit_lease,
            metrics: billing_context.metrics,
        },
        |mut state| async move {
            if !state.emit_done {
                return None;
            }

            match state.upstream.next().await {
                Some(upstream_chunk) => {
                    let event = match upstream_chunk {
                        Ok(upstream_chunk) => {
                            if let Some(usage) = upstream_chunk.usage {
                                state.latest_usage = Some(usage);
                            }

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
                            let status = app_error_status_code(&error);
                            let error_code = error_code_from_app_error(&error);
                            let usage = state.latest_usage.take().unwrap_or_else(|| {
                                billing::estimate_usage(&state.request_messages, "")
                            });
                            let latency_ms = state.request_started_at.elapsed().as_millis() as u64;
                            if let Err(error) = billing::record_usage(
                                &state.database,
                                state.database_backend,
                                billing::BillingInput {
                                    request_id: state.context.request_id,
                                    user_id: state.user_id,
                                    api_key_id: state.api_key_id,
                                    provider_id: Some(state.provider_id),
                                    route_id: Some(state.route_id),
                                    model: state.model.clone(),
                                    usage,
                                    status_code: status.as_u16(),
                                    latency_ms,
                                    route_price: state.route_price,
                                },
                            )
                            .instrument(info_span!(
                                "metering",
                                request_id = %state.context.request_id,
                                trace_id = %state.context.trace_id,
                                route = %state.route_alias,
                                status = status.as_u16(),
                            ))
                            .await
                            {
                                let usage_error_code = error_code_from_app_error(&error);
                                let usage_status = app_error_status_code(&error);
                                warn!(
                                    request_id = %state.context.request_id,
                                    error = %error,
                                    "failed to persist stream request usage after stream chunk error"
                                );
                                let _ = record_gateway_request_completion(
                                    &state.database,
                                    state.database_backend,
                                    &state.context,
                                    &state.request_started_at,
                                    usage_status,
                                    Some(&state.route_alias),
                                    state.context.provider.as_deref(),
                                    Some(usage_error_code.as_str()),
                                )
                                .await;
                                observe_gateway_metrics(
                                    &state.metrics,
                                    &state.context,
                                    &state.route_alias,
                                    usage,
                                    app_error_status_code(&error),
                                    latency_ms,
                                    state.route_price,
                                );
                                release_limit_lease(state.limit_lease.take());
                                return Some((
                                    Ok(Event::default()
                                        .event("error")
                                        .json_data(ErrorEnvelope::from(&error))
                                        .expect("error envelope should serialize")),
                                    state,
                                ));
                            }
                            observe_gateway_metrics(
                                &state.metrics,
                                &state.context,
                                &state.route_alias,
                                usage,
                                status,
                                latency_ms,
                                state.route_price,
                            );
                            let _ = record_gateway_request_completion(
                                &state.database,
                                state.database_backend,
                                &state.context,
                                &state.request_started_at,
                                status,
                                Some(&state.route_alias),
                                state.context.provider.as_deref(),
                                Some(error_code.as_str()),
                            )
                            .await;
                            release_limit_lease(state.limit_lease.take());
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
                        let usage = state.latest_usage.take().unwrap_or_else(|| {
                            billing::estimate_usage(&state.request_messages, "")
                        });
                        let latency_ms = state.request_started_at.elapsed().as_millis() as u64;
                        if let Err(error) = billing::record_usage(
                            &state.database,
                            state.database_backend,
                            billing::BillingInput {
                                request_id: state.context.request_id,
                                user_id: state.user_id,
                                api_key_id: state.api_key_id,
                                provider_id: Some(state.provider_id),
                                route_id: Some(state.route_id),
                                model: state.model.clone(),
                                usage,
                                status_code: StatusCode::OK.as_u16(),
                                latency_ms,
                                route_price: state.route_price,
                            },
                        )
                        .instrument(info_span!(
                            "metering",
                            request_id = %state.context.request_id,
                            trace_id = %state.context.trace_id,
                            route = %state.route_alias,
                            status = StatusCode::OK.as_u16(),
                        ))
                        .await
                        {
                            warn!(
                                request_id = %state.context.request_id,
                                error = %error,
                                "failed to persist stream request usage"
                            );
                            let _ = record_gateway_request_completion(
                                &state.database,
                                state.database_backend,
                                &state.context,
                                &state.request_started_at,
                                app_error_status_code(&error),
                                Some(&state.route_alias),
                                state.context.provider.as_deref(),
                                Some(error_code_from_app_error(&error).as_str()),
                            )
                            .await;
                            observe_gateway_metrics(
                                &state.metrics,
                                &state.context,
                                &state.route_alias,
                                usage,
                                app_error_status_code(&error),
                                latency_ms,
                                state.route_price,
                            );
                            release_limit_lease(state.limit_lease.take());
                            return Some((
                                Ok(Event::default()
                                    .event("error")
                                    .json_data(ErrorEnvelope::from(&error))
                                    .expect("error envelope should serialize")),
                                state,
                            ));
                        }
                        observe_gateway_metrics(
                            &state.metrics,
                            &state.context,
                            &state.route_alias,
                            usage,
                            StatusCode::OK,
                            latency_ms,
                            state.route_price,
                        );
                        let _ = record_gateway_request_completion(
                            &state.database,
                            state.database_backend,
                            &state.context,
                            &state.request_started_at,
                            StatusCode::OK,
                            Some(&state.route_alias),
                            state.context.provider.as_deref(),
                            None,
                        )
                        .await;
                        release_limit_lease(state.limit_lease.take());
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
    request_model_field: &'static str,
    public_model: &str,
) -> Result<ResolvedModelRoute, AppError> {
    #[derive(Debug, FromRow)]
    struct ResolvedModelRouteRow {
        #[sqlx(rename = "route_id")]
        id: String,
        upstream_model: String,
        provider_type: String,
        #[sqlx(rename = "provider_connection_id")]
        provider_connection_id: String,
        provider_base_url: String,
        #[sqlx(rename = "provider_api_key_encrypted")]
        provider_api_key_encrypted: String,
        max_tokens: Option<i64>,
        input_price_per_1m_tokens: i64,
        output_price_per_1m_tokens: i64,
    }

    let resolved = query_as::<_, ResolvedModelRouteRow>(&prepare_sql(
        database_backend,
        "SELECT mr.id AS route_id,
                mr.upstream_model,
                pc.provider_type,
                pc.id AS provider_connection_id,
                pc.base_url AS provider_base_url,
                pc.api_key_encrypted AS provider_api_key_encrypted,
                mr.max_tokens,
                mr.pricing_input_per_1m_tokens AS input_price_per_1m_tokens,
                mr.pricing_output_per_1m_tokens AS output_price_per_1m_tokens
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
    .ok_or_else(|| AppError::invalid_config(request_model_field, "model not found or disabled"))?;

    let route_id = resolved.id;
    let upstream_model = resolved.upstream_model;
    let provider_type = resolved.provider_type;
    let provider_connection_id = resolved.provider_connection_id;
    let provider_base_url = resolved.provider_base_url;
    let encrypted_api_key = resolved.provider_api_key_encrypted;
    let max_tokens = resolved.max_tokens;
    let input_price_per_1m_tokens = resolved.input_price_per_1m_tokens;
    let output_price_per_1m_tokens = resolved.output_price_per_1m_tokens;
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
    let max_tokens = max_tokens
        .map(|value| {
            u64::try_from(value).map_err(|_| {
                AppError::invalid_config("model_route.max_tokens", "max_tokens cannot be negative")
            })
        })
        .transpose()?;

    Ok(ResolvedModelRoute {
        id,
        provider_connection_id,
        upstream_model,
        provider_type: provider_type.trim().to_string(),
        provider_base_url,
        provider_api_key,
        max_tokens,
        input_price_per_1m_tokens,
        output_price_per_1m_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_limits::{RuntimeLimitPolicy, RuntimeLimitRequest};
    use redis::Commands;

    async fn sqlite_test_database() -> AnyPool {
        sqlx::any::install_default_drivers();
        sqlx::any::AnyPoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create sqlite test database")
    }

    fn redis_client_from_env() -> redis::RedisResult<redis::Client> {
        let redis_url =
            std::env::var("MIZAN_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        redis::Client::open(redis_url)
    }

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

    #[test]
    fn resolve_effective_max_tokens_uses_route_default_and_rejects_overrides() {
        assert_eq!(
            resolve_effective_max_tokens(None, Some(128), CHAT_COMPLETIONS_MAX_TOKENS_FIELD)
                .unwrap(),
            Some(128)
        );
        assert_eq!(
            resolve_effective_max_tokens(Some(64), Some(128), CHAT_COMPLETIONS_MAX_TOKENS_FIELD)
                .unwrap(),
            Some(64)
        );
        assert!(
            resolve_effective_max_tokens(Some(129), Some(128), CHAT_COMPLETIONS_MAX_TOKENS_FIELD)
                .is_err()
        );
        assert_eq!(
            resolve_effective_max_tokens(Some(64), None, CHAT_COMPLETIONS_MAX_TOKENS_FIELD)
                .unwrap(),
            Some(64)
        );
    }

    #[test]
    fn estimate_admission_usage_counts_expected_completion_tokens() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "hello world".to_string(),
        }];

        let usage = estimate_admission_usage(&messages, Some(100));

        assert!(usage.estimated);
        assert_eq!(usage.prompt_tokens, 3);
        assert_eq!(usage.completion_tokens, 100);
        assert_eq!(usage.total_tokens, 103);
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
        let database = sqlite_test_database().await;
        let events = build_stream_events(
            "chatcmpl-test",
            "mizan-public-model".to_string(),
            upstream,
            vec![],
            &context,
            StreamBillingContext {
                request_started_at: Instant::now(),
                database,
                database_backend: DatabaseBackend::Sqlite,
                route_price: RoutePrice {
                    input_microcredits_per_1m_tokens: 0,
                    output_microcredits_per_1m_tokens: 0,
                },
                user_id: Uuid::now_v7(),
                api_key_id: Some(Uuid::now_v7()),
                provider_id: Uuid::now_v7(),
                route_id: Uuid::now_v7(),
                limit_lease: None,
                metrics: MetricsRegistry::default(),
            },
        );
        futures_util::pin_mut!(events);

        let first = tokio::time::timeout(std::time::Duration::from_millis(100), events.next())
            .await
            .expect("gateway should emit before upstream closes")
            .expect("gateway stream should yield an event")
            .expect("event should be infallible");

        assert!(format!("{first:?}").contains("chat.completion.chunk"));
    }

    #[tokio::test]
    async fn build_stream_events_does_not_emit_done_after_stream_error() {
        let upstream = stream::iter([Err(AppError::provider("stream failure"))]).boxed();
        let context = RequestContext::new();
        let database = sqlite_test_database().await;
        let events = build_stream_events(
            "chatcmpl-error",
            "mizan-public-model".to_string(),
            upstream,
            vec![],
            &context,
            StreamBillingContext {
                request_started_at: Instant::now(),
                database,
                database_backend: DatabaseBackend::Sqlite,
                route_price: RoutePrice {
                    input_microcredits_per_1m_tokens: 0,
                    output_microcredits_per_1m_tokens: 0,
                },
                user_id: Uuid::now_v7(),
                api_key_id: Some(Uuid::now_v7()),
                provider_id: Uuid::now_v7(),
                route_id: Uuid::now_v7(),
                limit_lease: None,
                metrics: MetricsRegistry::default(),
            },
        );
        futures_util::pin_mut!(events);

        let first = tokio::time::timeout(std::time::Duration::from_millis(100), events.next())
            .await
            .expect("gateway should emit stream error event")
            .expect("gateway stream should yield an event")
            .expect("event should be infallible");

        assert!(format!("{first:?}").contains("error"));

        let second = tokio::time::timeout(std::time::Duration::from_millis(100), events.next())
            .await
            .expect("stream should finish after error");

        assert!(second.is_none());
    }

    #[tokio::test]
    #[ignore = "requires a reachable Redis instance; run with scripts/limit-smoke.sh"]
    async fn dropping_stream_events_releases_runtime_limit_lease() {
        let client = redis_client_from_env().expect("redis client");
        let scope_id = Uuid::now_v7();
        let scope = LimitScope::Provider(scope_id);
        let concurrency_key = mizan_limits::concurrency_counter_key(scope);

        let mut connection = client.get_connection().expect("redis connection");
        redis::cmd("DEL")
            .arg(&concurrency_key)
            .query::<i64>(&mut connection)
            .expect("cleanup concurrency key");

        let limit_lease = mizan_limits::check_and_acquire(
            client.clone(),
            RuntimeLimitPolicy {
                requests_per_window: 0,
                tokens_per_window: 0,
                concurrent_requests: 1,
                window_seconds: 60,
                lease_seconds: 30,
            },
            RuntimeLimitRequest {
                scopes: vec![scope],
                estimated_prompt_tokens: 0,
            },
        )
        .expect("lease should be acquired");

        let upstream = stream::iter([Ok(ChatStreamChunk {
            index: 0,
            delta: "hello".to_string(),
            finish_reason: None,
            usage: None,
        })])
        .boxed();
        let context = RequestContext::new();
        let database = sqlite_test_database().await;
        let mut events = Box::pin(build_stream_events(
            "chatcmpl-drop",
            "mizan-public-model".to_string(),
            upstream,
            vec![],
            &context,
            StreamBillingContext {
                request_started_at: Instant::now(),
                database,
                database_backend: DatabaseBackend::Sqlite,
                route_price: RoutePrice {
                    input_microcredits_per_1m_tokens: 0,
                    output_microcredits_per_1m_tokens: 0,
                },
                user_id: Uuid::now_v7(),
                api_key_id: Some(Uuid::now_v7()),
                provider_id: scope_id,
                route_id: Uuid::now_v7(),
                limit_lease: Some(limit_lease),
                metrics: MetricsRegistry::default(),
            },
        ));

        let first = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            events.as_mut().next(),
        )
        .await
        .expect("gateway should emit first stream event")
        .expect("gateway stream should yield an event")
        .expect("event should be infallible");
        assert!(format!("{first:?}").contains("hello"));

        drop(events);
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            let remaining: Option<i64> = connection
                .get(&concurrency_key)
                .expect("get concurrency key");
            if remaining.is_none() {
                return;
            }
        }

        panic!("dropped stream should release runtime limit lease");
    }
}
