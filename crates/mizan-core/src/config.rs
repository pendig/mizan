use std::{env, net::SocketAddr, path::PathBuf};

use crate::{AppError, AppResult, DatabaseBackend};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub http_addr: SocketAddr,
    pub database_backend: DatabaseBackend,
    pub database_url: String,
    pub run_migrations: bool,
    pub redis_url: String,
    pub log_level: String,
    pub admin_seed_email: Option<String>,
    pub admin_seed_password: Option<String>,
    pub admin_seed_role: String,
}

impl AppConfig {
    pub fn from_env() -> AppResult<Self> {
        let http_addr = env::var("MIZAN_HTTP_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:18180".to_owned())
            .parse()
            .map_err(|err| AppError::config("MIZAN_HTTP_ADDR", err))?;

        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            let data_dir = env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("data")
                .join("mizan.sqlite3");
            format!("sqlite://{}?mode=rwc", data_dir.to_string_lossy())
        });
        let database_url = normalize_sqlite_url(database_url)?;

        let admin_seed_email = env::var("MIZAN_ADMIN_EMAIL")
            .ok()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        let admin_seed_password = env::var("MIZAN_ADMIN_PASSWORD")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let admin_seed_role = env::var("MIZAN_ADMIN_ROLE")
            .unwrap_or_else(|_| "admin".to_owned())
            .trim()
            .to_lowercase();
        let admin_seed_role = if admin_seed_role.is_empty() {
            "admin".to_owned()
        } else {
            admin_seed_role
        };

        if matches!(
            (admin_seed_email.as_deref(), admin_seed_password.as_deref()),
            (Some(_), Some(_)) | (None, None)
        ) {
            Ok(Self {
                http_addr,
                database_backend: DatabaseBackend::from_url(&database_url)?,
                database_url,
                run_migrations: env::var("MIZAN_RUN_MIGRATIONS")
                    .unwrap_or_else(|_| "true".to_owned())
                    .parse()
                    .unwrap_or(true),
                redis_url: env::var("REDIS_URL")
                    .unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_owned()),
                log_level: env::var("RUST_LOG")
                    .unwrap_or_else(|_| "mizan=info,tower_http=info".to_owned()),
                admin_seed_email,
                admin_seed_password,
                admin_seed_role,
            })
        } else {
            Err(AppError::invalid_config(
                "MIZAN_ADMIN_EMAIL",
                "set both MIZAN_ADMIN_EMAIL and MIZAN_ADMIN_PASSWORD, or set neither",
            ))
        }
    }
}

fn normalize_sqlite_url(database_url: String) -> AppResult<String> {
    if !database_url.starts_with("sqlite://") {
        return Ok(database_url);
    }

    let trimmed = database_url.trim_start_matches("sqlite://");
    let mut parts = trimmed.splitn(2, '?');
    let path_part = parts.next().unwrap_or_default();
    let query_part = parts.next();

    if path_part == ":memory:" {
        return Ok(database_url);
    }

    let path = if path_part.starts_with('/') {
        PathBuf::from(path_part)
    } else {
        env::current_dir()
            .map_err(|error| AppError::infrastructure(error.to_string()))?
            .join(path_part)
    };

    let mut normalized = String::from("sqlite://");
    normalized.push_str(&path.to_string_lossy());

    match query_part {
        Some(query) if !query.is_empty() => {
            normalized.push('?');
            normalized.push_str(query);
        }
        Some(_) => {}
        None => {
            normalized.push_str("?mode=rwc");
        }
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_relative_sqlite_url_to_absolute_path() {
        let normalized =
            normalize_sqlite_url("sqlite://./data/test.sqlite3".to_string()).expect("normalize");

        assert!(normalized.starts_with("sqlite://"));
        assert!(normalized.contains("?mode=rwc"));
        assert!(normalized.ends_with("data/test.sqlite3?mode=rwc"));
    }
}
