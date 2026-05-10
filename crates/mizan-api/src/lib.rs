use axum::{Json, Router, extract::State, response::IntoResponse, routing::get};
use mizan_core::{AppConfig, AppError, AppResult, DatabaseBackend, ErrorEnvelope, init_tracing};
use mizan_gateway::Gateway;
use redis::Client as RedisClient;
use serde::Serialize;
use sqlx::{AnyPool, query_scalar};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

mod storage;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub gateway: Gateway,
    pub database: AnyPool,
    pub redis: RedisClient,
}

impl AppState {
    pub async fn new(config: AppConfig) -> AppResult<Self> {
        let database =
            storage::connect_and_migrate(&config.database_url, config.run_migrations).await?;

        let redis = RedisClient::open(config.redis_url.as_str())
            .map_err(|err| AppError::infrastructure(err.to_string()))?;

        Ok(Self {
            config,
            gateway: Gateway::new(),
            database,
            redis,
        })
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
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
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

async fn readyz(State(state): State<AppState>) -> Json<HealthResponse> {
    let database_ok = state.check_database().await;
    let status = if database_ok { "ready" } else { "not_ready" };
    let dependencies = HealthDependencies {
        database_backend: state.database_backend().as_str(),
        database: if database_ok { "ok" } else { "not_ready" },
        redis: "not_checked",
    };

    let response = HealthResponse {
        status,
        service: "mizan-api",
        version: env!("CARGO_PKG_VERSION"),
        dependencies,
    };

    Json(response)
}

async fn not_found() -> impl IntoResponse {
    error!("unmatched route");
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
