use std::{env, net::SocketAddr, path::PathBuf};

use crate::{AppError, AppResult, DatabaseBackend};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub http_addr: SocketAddr,
    pub database_backend: DatabaseBackend,
    pub database_url: String,
    pub database_max_connections: u32,
    pub run_migrations: bool,
    pub redis_url: String,
    pub limit_rpm: u32,
    pub limit_tpm: u32,
    pub limit_concurrency: u32,
    pub limit_window_seconds: u32,
    pub limit_lease_seconds: u32,
    pub log_level: String,
    pub admin_seed_email: Option<String>,
    pub admin_seed_password: Option<String>,
    pub admin_seed_role: String,
    pub provider_secret_key: Option<String>,
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
        let provider_secret_key = env::var("MIZAN_PROVIDER_SECRET_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        if matches!(
            (admin_seed_email.as_deref(), admin_seed_password.as_deref()),
            (Some(_), Some(_)) | (None, None)
        ) {
            let database_max_connections = env::var("MIZAN_DB_MAX_CONNECTIONS").map_or(
                Ok(DEFAULT_DATABASE_MAX_CONNECTIONS),
                |value| {
                    parse_u32_env(
                        "MIZAN_DB_MAX_CONNECTIONS",
                        &value,
                        DEFAULT_DATABASE_MAX_CONNECTIONS,
                    )
                },
            )?;

            Ok(Self {
                http_addr,
                database_backend: DatabaseBackend::from_url(&database_url)?,
                database_url,
                database_max_connections,
                run_migrations: parse_bool_env("MIZAN_RUN_MIGRATIONS", "true", |value| {
                    parse_bool_value("MIZAN_RUN_MIGRATIONS", value)
                })?,
                redis_url: env::var("REDIS_URL")
                    .unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_owned()),
                limit_rpm: env::var("MIZAN_LIMIT_RPM")
                    .map_or(Ok(0), |value| parse_u32_env("MIZAN_LIMIT_RPM", &value, 0))?,
                limit_tpm: env::var("MIZAN_LIMIT_TPM")
                    .map_or(Ok(0), |value| parse_u32_env("MIZAN_LIMIT_TPM", &value, 0))?,
                limit_concurrency: env::var("MIZAN_LIMIT_CONCURRENCY").map_or(Ok(0), |value| {
                    parse_u32_env("MIZAN_LIMIT_CONCURRENCY", &value, 0)
                })?,
                limit_window_seconds: env::var("MIZAN_LIMIT_WINDOW_SECONDS").map_or(
                    Ok(DEFAULT_LIMIT_WINDOW_SECONDS),
                    |value| {
                        parse_u32_env(
                            "MIZAN_LIMIT_WINDOW_SECONDS",
                            &value,
                            DEFAULT_LIMIT_WINDOW_SECONDS,
                        )
                    },
                )?,
                limit_lease_seconds: env::var("MIZAN_LIMIT_LEASE_SECONDS").map_or(
                    Ok(DEFAULT_LIMIT_LEASE_SECONDS),
                    |value| {
                        parse_u32_env(
                            "MIZAN_LIMIT_LEASE_SECONDS",
                            &value,
                            DEFAULT_LIMIT_LEASE_SECONDS,
                        )
                    },
                )?,
                log_level: env::var("RUST_LOG")
                    .unwrap_or_else(|_| "mizan=info,tower_http=info".to_owned()),
                admin_seed_email,
                admin_seed_password,
                admin_seed_role,
                provider_secret_key,
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
    if !database_url.starts_with("sqlite://") && !database_url.starts_with("sqlite:") {
        return Ok(database_url);
    }

    let trimmed = if let Some(trimmed) = database_url.strip_prefix("sqlite://") {
        trimmed
    } else {
        database_url
            .strip_prefix("sqlite:")
            .unwrap_or(&database_url)
    };
    let mut parts = trimmed.splitn(2, '?');
    let path_part = parts.next().unwrap_or_default();
    let query_part = parts.next();

    if path_part == ":memory:" || path_part == "file::memory:" {
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

fn parse_bool_env(
    key: &'static str,
    default: &str,
    parser: impl Fn(&str) -> AppResult<bool>,
) -> AppResult<bool> {
    let value = env::var(key).unwrap_or_else(|_| default.to_owned());
    parser(value.as_str())
}

fn parse_bool_value(key: &'static str, raw_value: &str) -> AppResult<bool> {
    match raw_value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "t" | "yes" | "y" | "on" => Ok(true),
        "0" | "false" | "f" | "no" | "n" | "off" => Ok(false),
        "" => Err(AppError::invalid_config(
            key,
            "expected a boolean value: true, false, 1, 0, yes, no",
        )),
        _ => Err(AppError::invalid_config(
            key,
            "invalid boolean value, use true|false|1|0|yes|no|on|off",
        )),
    }
}

const DEFAULT_DATABASE_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_LIMIT_WINDOW_SECONDS: u32 = 60;
const DEFAULT_LIMIT_LEASE_SECONDS: u32 = 120;

fn parse_u32_env(key: &'static str, raw_value: &str, default: u32) -> AppResult<u32> {
    let value = raw_value.trim();
    if value.is_empty() {
        return Ok(default);
    }

    let parsed = value
        .parse::<u32>()
        .map_err(|_| AppError::invalid_config(key, "must be a positive integer"))?;

    if parsed == 0 && default > 0 {
        return Err(AppError::invalid_config(key, "must be greater than zero"));
    }

    Ok(parsed)
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

    #[test]
    fn normalizes_legacy_sqlite_url_to_absolute_path() {
        let normalized =
            normalize_sqlite_url("sqlite:./data/legacy.sqlite3".to_string()).expect("normalize");

        assert!(normalized.starts_with("sqlite://"));
        assert!(normalized.contains("?mode=rwc"));
        assert!(normalized.ends_with("data/legacy.sqlite3?mode=rwc"));
    }

    #[test]
    fn preserves_sqlite_memory_urls_during_normalization() {
        let with_legacy = normalize_sqlite_url("sqlite:file::memory:?cache=shared".to_string())
            .expect("normalize");
        assert_eq!(with_legacy, "sqlite:file::memory:?cache=shared");

        let with_double_slash =
            normalize_sqlite_url("sqlite://file::memory:?cache=shared".to_string())
                .expect("normalize");
        assert_eq!(with_double_slash, "sqlite://file::memory:?cache=shared");
    }

    #[test]
    fn parse_bool_env_uses_default_when_missing() {
        let value = parse_bool_value("MIZAN_RUN_MIGRATIONS", "true").expect("true");
        assert!(value);
        let value = parse_bool_value("MIZAN_RUN_MIGRATIONS", "0").expect("false");
        assert!(!value);
        assert!(parse_bool_value("MIZAN_RUN_MIGRATIONS", "").is_err());
    }

    #[test]
    fn parse_u32_env_rejects_zero_or_invalid_values() {
        assert!(parse_u32_env("MIZAN_DB_MAX_CONNECTIONS", "10", 10).expect("ok") == 10);
        assert!(parse_u32_env("MIZAN_DB_MAX_CONNECTIONS", "0", 10).is_err());
        assert!(parse_u32_env("MIZAN_DB_MAX_CONNECTIONS", "bad", 10).is_err());
    }

    #[test]
    fn parse_u32_env_allows_zero_when_default_is_zero() {
        assert_eq!(
            parse_u32_env("MIZAN_LIMIT_RPM", "0", 0).expect("disabled"),
            0
        );
        assert_eq!(parse_u32_env("MIZAN_LIMIT_RPM", "", 0).expect("default"), 0);
    }
}
