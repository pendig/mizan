use std::fmt;

use crate::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseBackend {
    Sqlite,
    Postgres,
}

impl DatabaseBackend {
    pub fn from_url(value: &str) -> Result<Self, AppError> {
        if value.starts_with("sqlite:") {
            return Ok(Self::Sqlite);
        }

        if value.starts_with("postgres://") || value.starts_with("postgresql://") {
            return Ok(Self::Postgres);
        }

        Err(AppError::invalid_config(
            "DATABASE_URL",
            format!("unsupported database URL scheme: {value}"),
        ))
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgresql",
        }
    }
}

impl fmt::Display for DatabaseBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
