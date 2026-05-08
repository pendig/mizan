use axum::{Json, Router, extract::State, routing::get};
use mizan_core::{AppConfig, AppError, AppResult, init_tracing};
use mizan_gateway::Gateway;
use redis::Client as RedisClient;
use serde::Serialize;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub gateway: Gateway,
    pub postgres: PgPool,
    pub redis: RedisClient,
}

impl AppState {
    pub fn new(config: AppConfig) -> AppResult<Self> {
        let postgres = PgPoolOptions::new()
            .max_connections(5)
            .connect_lazy(&config.database_url)
            .map_err(AppError::infrastructure)?;
        let redis =
            RedisClient::open(config.redis_url.as_str()).map_err(AppError::infrastructure)?;

        Ok(Self {
            config,
            gateway: Gateway::new(),
            postgres,
            redis,
        })
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

pub async fn run_from_env() -> AppResult<()> {
    let config = AppConfig::from_env()?;
    init_tracing(&config.log_level)?;

    let listener = TcpListener::bind(config.http_addr).await?;
    let state = AppState::new(config.clone())?;

    info!(addr = %config.http_addr, "starting mizan api");

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(AppError::infrastructure)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn healthz(State(state): State<AppState>) -> Json<HealthResponse> {
    let health = state.gateway.health();

    Json(HealthResponse {
        status: health.status,
        service: "mizan-api",
        version: env!("CARGO_PKG_VERSION"),
    })
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
