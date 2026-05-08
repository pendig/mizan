use std::{error::Error as StdError, fmt};

use serde::Serialize;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("configuration error for {key}: {source}")]
    Config {
        key: &'static str,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },

    #[error("infrastructure error: {0}")]
    Infrastructure(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error("limit exceeded: {0}")]
    LimitExceeded(String),

    #[error("insufficient credit")]
    InsufficientCredit,

    #[error("upstream provider error: {0}")]
    Provider(String),
}

impl AppError {
    pub fn config(key: &'static str, source: impl StdError + Send + Sync + 'static) -> Self {
        Self::Config {
            key,
            source: Box::new(source),
        }
    }

    pub fn infrastructure(message: impl fmt::Display) -> Self {
        Self::Infrastructure(message.to_string())
    }

    pub fn provider(message: impl fmt::Display) -> Self {
        Self::Provider(message.to_string())
    }

    pub fn public_code(&self) -> &'static str {
        match self {
            Self::Config { .. } => "configuration_error",
            Self::Infrastructure(_) => "infrastructure_error",
            Self::Io(_) => "io_error",
            Self::NotFound(_) => "not_found",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::LimitExceeded(_) => "limit_exceeded",
            Self::InsufficientCredit => "insufficient_credit",
            Self::Provider(_) => "provider_error",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorEnvelope {
    pub code: &'static str,
    pub message: String,
}

impl From<&AppError> for ErrorEnvelope {
    fn from(error: &AppError) -> Self {
        Self {
            code: error.public_code(),
            message: error.to_string(),
        }
    }
}
