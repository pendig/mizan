use axum::http::StatusCode;
use axum::{Extension, Json, extract::State};
use mizan_core::{AppError, DatabaseBackend, ErrorEnvelope, RequestContextBuilder};
use mizan_providers::{ChatMessage, ChatRequest, ChatResponse, OpenAiCompatibleProvider};
use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, query_as};
use uuid::Uuid;

use crate::AppState;
use crate::auth::ApiKeyIdentity;

type GatewayHttpResult<T> = Result<T, (StatusCode, Json<ErrorEnvelope>)>;

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

#[derive(Debug)]
struct ResolvedModelRoute {
    id: Uuid,
    upstream_model: String,
    provider_type: String,
}

pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
    Json(payload): Json<ChatCompletionsRequest>,
) -> GatewayHttpResult<Json<ChatCompletionsResponse>> {
    let public_model = payload.model.trim();
    if public_model.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "chat_completion.model",
                "model is required",
            ))),
        ));
    }

    let route = resolve_model_route(&state.database, state.database_backend(), public_model)
        .await
        .map_err(from_app_error)?;

    let context = RequestContextBuilder::default()
        .user_id(identity.user_id)
        .api_key_id(identity.api_key_id)
        .provider(route.provider_type.clone())
        .route(public_model.to_string())
        .route_id(route.id)
        .model(route.upstream_model.clone())
        .streaming(payload.stream)
        .build();

    let upstream_request = ChatRequest {
        model: route.upstream_model.clone(),
        messages: payload.messages.clone(),
        stream: payload.stream,
    };

    let provider_name = if route.provider_type.eq_ignore_ascii_case("openai") {
        "openai"
    } else {
        "openai-compatible"
    };
    let provider = OpenAiCompatibleProvider::new(provider_name);
    let upstream_response = state
        .gateway
        .chat_completions(&context, &provider, upstream_request)
        .await
        .map_err(from_app_error)?;

    Ok(Json(map_to_chat_completion_response(
        route.upstream_model,
        upstream_response,
    )))
}

fn map_to_chat_completion_response(
    model: String,
    upstream: ChatResponse,
) -> ChatCompletionsResponse {
    ChatCompletionsResponse {
        id: format!("chatcmpl-{}", Uuid::now_v7()),
        object: "chat.completion",
        created: now_utc_epoch_seconds() * 1000,
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

async fn resolve_model_route(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    public_model: &str,
) -> Result<ResolvedModelRoute, AppError> {
    let resolved = query_as::<_, (String, String, String)>(&prepare_sql(
        database_backend,
        "SELECT mr.id,
                mr.upstream_model,
                pc.provider_type
         FROM model_routes mr
         INNER JOIN provider_connections pc
            ON pc.id = mr.provider_connection_id
         WHERE mr.public_model = ? AND mr.enabled = 1 AND pc.enabled = 1",
    ))
    .bind(public_model)
    .fetch_optional(database)
    .await
    .map_err(|error| AppError::infrastructure(error.to_string()))?
    .ok_or_else(|| {
        AppError::invalid_config("chat_completion.model", "model not found or disabled")
    })?;

    let (route_id, upstream_model, provider_type) = resolved;
    let id = Uuid::parse_str(&route_id).map_err(|error| {
        AppError::infrastructure(format!("stored route id is invalid: {error}"))
    })?;

    Ok(ResolvedModelRoute {
        id,
        upstream_model,
        provider_type: provider_type.trim().to_string(),
    })
}

fn from_app_error(error: AppError) -> (StatusCode, Json<ErrorEnvelope>) {
    let status = match error {
        AppError::InvalidConfig { .. } => StatusCode::BAD_REQUEST,
        AppError::NotFound(_) => StatusCode::NOT_FOUND,
        AppError::Unauthorized => StatusCode::UNAUTHORIZED,
        AppError::Forbidden => StatusCode::FORBIDDEN,
        AppError::Provider(_) => StatusCode::BAD_GATEWAY,
        AppError::LimitExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
        AppError::InsufficientCredit => StatusCode::PAYMENT_REQUIRED,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };

    (status, Json(ErrorEnvelope::from(&error)))
}

fn now_utc_epoch_seconds() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time moved backwards")
        .as_secs() as i64
}

fn prepare_sql(database_backend: DatabaseBackend, query: &'static str) -> String {
    match database_backend {
        DatabaseBackend::Sqlite => query.to_string(),
        DatabaseBackend::Postgres => to_dollar_params(query),
    }
}

fn to_dollar_params(query: &str) -> String {
    let mut index = 0usize;
    let mut converted = String::with_capacity(query.len());

    for character in query.chars() {
        if character == '?' {
            index += 1;
            converted.push('$');
            converted.push_str(&index.to_string());
            continue;
        }

        converted.push(character);
    }

    converted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_to_chat_completion_response_uses_model_and_content() {
        let model = "openai/gpt-4o-mini".to_string();
        let upstream = ChatResponse {
            provider: "openai".to_string(),
            model: model.clone(),
            content: "pong".to_string(),
            usage: Some(mizan_providers::TokenUsage {
                prompt_tokens: 7,
                completion_tokens: 3,
                total_tokens: 10,
                estimated: false,
            }),
        };

        let response = map_to_chat_completion_response(model.clone(), upstream);
        assert_eq!(response.model, model);
        assert_eq!(response.choices.len(), 1);
        assert_eq!(response.choices[0].message.content, "pong");
    }
}
