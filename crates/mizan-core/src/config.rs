use std::{env, net::SocketAddr};

use crate::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub http_addr: SocketAddr,
    pub database_url: String,
    pub redis_url: String,
    pub log_level: String,
}

impl AppConfig {
    pub fn from_env() -> AppResult<Self> {
        let http_addr = env::var("MIZAN_HTTP_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
            .parse()
            .map_err(|err| AppError::config("MIZAN_HTTP_ADDR", err))?;

        Ok(Self {
            http_addr,
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://mizan:mizan@localhost:5432/mizan".to_owned()),
            redis_url: env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_owned()),
            log_level: env::var("RUST_LOG")
                .unwrap_or_else(|_| "mizan=info,tower_http=info".to_owned()),
        })
    }
}
