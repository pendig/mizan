use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{AppError, AppResult};

pub fn init_tracing(default_filter: &str) -> AppResult<()> {
    let filter = EnvFilter::try_new(default_filter)
        .or_else(|_| EnvFilter::try_new("mizan=info,tower_http=info"))
        .map_err(|err| AppError::config("RUST_LOG", err))?;

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().json())
        .try_init()
        .map_err(AppError::infrastructure)
}
