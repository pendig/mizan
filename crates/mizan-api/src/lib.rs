use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    middleware::{from_fn, from_fn_with_state},
    response::IntoResponse,
    routing::{delete, get, post},
};
use mizan_core::{AppConfig, AppError, AppResult, DatabaseBackend, ErrorEnvelope, init_tracing};
use mizan_gateway::Gateway;
use redis::Client as RedisClient;
use serde::Serialize;
use sqlx::{AnyPool, query_scalar};
use tokio::net::TcpListener;
use tokio::task;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

mod auth;
mod billing;
mod gateway;
mod providers;
mod storage;
mod utils;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub gateway: Gateway,
    pub database: AnyPool,
    pub redis: RedisClient,
}

impl AppState {
    pub async fn new(config: AppConfig) -> AppResult<Self> {
        let database = storage::connect_and_migrate(
            &config.database_url,
            config.run_migrations,
            config.database_max_connections,
        )
        .await?;

        let redis = RedisClient::open(config.redis_url.as_str())
            .map_err(|err| AppError::infrastructure(err.to_string()))?;

        let state = Self {
            config,
            gateway: Gateway::new(),
            database,
            redis,
        };

        if let (Some(email), Some(password)) = (
            state.config.admin_seed_email.as_deref(),
            state.config.admin_seed_password.as_deref(),
        ) {
            auth::ensure_admin_seed(
                &state.database,
                state.database_backend(),
                email,
                password,
                &state.config.admin_seed_role,
            )
            .await?;
        }

        Ok(state)
    }

    pub async fn check_database(&self) -> bool {
        query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.database)
            .await
            .is_ok_and(|value| value == 1)
    }

    pub fn database_backend(&self) -> DatabaseBackend {
        self.config.database_backend
    }

    pub async fn check_redis(&self) -> bool {
        let client = self.redis.clone();
        let result = task::spawn_blocking(move || -> AppResult<bool> {
            let mut connection = client
                .get_connection()
                .map_err(|error| AppError::infrastructure(error.to_string()))?;

            let response: String = redis::cmd("PING")
                .query(&mut connection)
                .map_err(|error| AppError::infrastructure(error.to_string()))?;

            Ok(response == "PONG")
        })
        .await;

        match result {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                warn!(error = %error, "redis ping failed");
                false
            }
            Err(error) => {
                warn!(error = %error, "redis ping task failed");
                false
            }
        }
    }
}

#[derive(Debug, Serialize)]
struct HealthDependencies {
    database_backend: &'static str,
    database: &'static str,
    redis: &'static str,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    version: &'static str,
    dependencies: HealthDependencies,
}

pub async fn run_from_env() -> AppResult<()> {
    let config = AppConfig::from_env()?;
    init_tracing(&config.log_level)?;

    let listener = TcpListener::bind(config.http_addr).await?;
    let state = AppState::new(config.clone()).await?;

    info!(addr = %config.http_addr, "starting mizan api");

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(AppError::infrastructure)
}

pub fn router(state: AppState) -> Router {
    let public_auth_router = Router::new()
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login));

    let session_router = Router::new()
        .route("/me", get(auth::me))
        .route("/auth/logout", post(auth::logout))
        .route(
            "/api-keys",
            post(auth::create_api_key).get(auth::list_api_keys),
        )
        .route("/api-keys/{id}", delete(auth::revoke_api_key));

    let api_key_router = Router::new()
        .route("/v1/ping", get(auth::api_key_ping))
        .route("/v1/chat/completions", post(gateway::chat_completions))
        .route_layer(from_fn_with_state(state.clone(), auth::api_key_auth));

    let public_models_router = Router::new()
        .route("/v1/models", get(providers::list_models))
        .route_layer(from_fn_with_state(state.clone(), auth::api_key_auth));

    let provider_admin_router = Router::new()
        .route(
            "/admin/provider-connections",
            get(providers::list_provider_connections).post(providers::create_provider_connection),
        )
        .route(
            "/admin/provider-connections/{id}",
            delete(providers::delete_provider_connection),
        )
        .route(
            "/admin/model-routes",
            get(providers::list_model_routes).post(providers::create_model_route),
        )
        .route(
            "/admin/model-routes/{id}",
            delete(providers::delete_model_route),
        )
        .route_layer(from_fn(providers::require_admin_role))
        .route_layer(from_fn_with_state(state.clone(), auth::api_key_auth));

    let provider_router = Router::new()
        .merge(public_models_router)
        .merge(provider_admin_router);

    let billing_router = Router::new()
        .route("/v1/credits", get(billing::get_wallet))
        .route("/v1/usage", get(billing::list_usage))
        .route_layer(from_fn_with_state(state.clone(), auth::api_key_auth));

    let billing_admin_router = Router::new()
        .route(
            "/admin/users/{id}/credits/grant",
            post(billing::grant_credits),
        )
        .route("/admin/usage", get(billing::list_usage_admin))
        .route_layer(from_fn(providers::require_admin_role))
        .route_layer(from_fn_with_state(state.clone(), auth::api_key_auth));

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .merge(public_auth_router)
        .merge(session_router)
        .merge(api_key_router)
        .merge(billing_router)
        .merge(provider_router)
        .merge(billing_admin_router)
        .fallback(not_found)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "mizan-api",
        version: env!("CARGO_PKG_VERSION"),
        dependencies: HealthDependencies {
            database_backend: "not_checked",
            database: "not_checked",
            redis: "not_checked",
        },
    })
}

async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let database_ok = state.check_database().await;
    let redis_ok = state.check_redis().await;
    let ready = database_ok && redis_ok;
    let dependencies = HealthDependencies {
        database_backend: state.database_backend().as_str(),
        database: if database_ok { "ok" } else { "not_ready" },
        redis: if redis_ok { "ok" } else { "not_ready" },
    };

    let status = if ready { "ready" } else { "not_ready" };
    let response = HealthResponse {
        status,
        service: "mizan-api",
        version: env!("CARGO_PKG_VERSION"),
        dependencies,
    };

    if ready {
        (StatusCode::OK, Json(response))
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(response))
    }
}

async fn not_found() -> impl IntoResponse {
    warn!("unmatched route");
    (
        axum::http::StatusCode::NOT_FOUND,
        Json(ErrorEnvelope::from(&AppError::NotFound(
            "not found".to_string(),
        ))),
    )
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
