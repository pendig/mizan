use axum::http::StatusCode;
use axum::{Extension, Json, extract::State};
use mizan_core::{AppError, DatabaseBackend, ErrorEnvelope, RequestContextBuilder};
use mizan_providers::{ChatMessage, ChatRequest, ChatResponse, OpenAiCompatibleProvider};
use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, query_as};
use uuid::Uuid;

use crate::AppState;
use crate::auth::ApiKeyIdentity;
use crate::utils::{decrypt_provider_api_key, from_app_error, now_utc_epoch_seconds, prepare_sql};

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
    provider_connection_id: Uuid,
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

    let route = resolve_model_route(
        &state.database,
        state.database_backend(),
        state.config.provider_secret_key.as_deref(),
        public_model,
    )
    .await
    .map_err(from_app_error)?;

    let context = RequestContextBuilder::default()
        .user_id(identity.user_id)
        .api_key_id(identity.api_key_id)
        .provider(route.provider_type.clone())
        .route(public_model.to_string())
        .route_id(route.id)
        .provider_id(route.provider_connection_id)
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

async fn resolve_model_route(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    provider_secret_key: Option<&str>,
    public_model: &str,
) -> Result<ResolvedModelRoute, AppError> {
    let resolved = query_as::<_, (String, String, String, String, String)>(&prepare_sql(
        database_backend,
        "SELECT mr.id,
                mr.upstream_model,
                pc.provider_type,
                pc.id,
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

    let (route_id, upstream_model, provider_type, provider_connection_id, encrypted_api_key) =
        resolved;
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
    let _provider_api_key = decrypt_provider_api_key(
        provider_secret_key,
        &provider_connection_id.to_string(),
        &encrypted_api_key,
    )?;

    Ok(ResolvedModelRoute {
        id,
        provider_connection_id,
        upstream_model,
        provider_type: provider_type.trim().to_string(),
    })
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
