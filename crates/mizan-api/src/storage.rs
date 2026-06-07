use std::path::Path;

use mizan_core::{AppError, AppResult};
use sqlx::{AnyPool, any::AnyPoolOptions, any::install_default_drivers, migrate::Migrator};

static MIGRATOR: Migrator = sqlx::migrate!("./../../migrations");

pub async fn connect_and_migrate(
    database_url: &str,
    run_migrations: bool,
    max_connections: u32,
) -> AppResult<AnyPool> {
    ensure_sqlite_parent_directory(database_url).await?;

    install_default_drivers();

    let max_connections = if is_sqlite_in_memory(database_url) {
        1
    } else {
        max_connections.max(1)
    };

    let pool = AnyPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
        .map_err(AppError::infrastructure)?;

    if run_migrations {
        MIGRATOR
            .run(&pool)
            .await
            .map_err(|error| AppError::infrastructure(error.to_string()))?;
    }

    Ok(pool)
}

async fn ensure_sqlite_parent_directory(database_url: &str) -> AppResult<()> {
    if !database_url.starts_with("sqlite://") && !database_url.starts_with("sqlite:") {
        return Ok(());
    }

    let path = if let Some(path) = database_url.strip_prefix("sqlite://") {
        path
    } else {
        database_url.strip_prefix("sqlite:").unwrap_or(database_url)
    };
    let path = path.split('?').next().unwrap_or(path);

    if path == ":memory:" || path.is_empty() {
        return Ok(());
    }

    let parent = Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    match parent {
        Some(parent) => {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                AppError::infrastructure(format!("cannot create sqlite data directory: {error}"))
            })?;
            Ok(())
        }
        None => Ok(()),
    }
}

fn is_sqlite_in_memory(database_url: &str) -> bool {
    if !database_url.starts_with("sqlite://") && !database_url.starts_with("sqlite:") {
        return false;
    }

    let path = if let Some(path) = database_url.strip_prefix("sqlite://") {
        path
    } else {
        database_url.strip_prefix("sqlite:").unwrap_or(database_url)
    };
    let path = path.split('?').next().unwrap_or(path);

    path == ":memory:" || path == "file::memory:"
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::query_scalar;
    use std::collections::BTreeSet;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_sqlite_database() -> (String, std::path::PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mizan-m2-{}.sqlite3", nanos));

        (
            format!("sqlite://{}?mode=rwc", path.to_string_lossy()),
            path,
        )
    }

    #[tokio::test]
    async fn migration_applies_idempotently_and_creates_schema_objects() {
        let (database_url, path) = temporary_sqlite_database();

        let _first_pool = connect_and_migrate(&database_url, true, 10)
            .await
            .expect("run migration once");
        let pool = connect_and_migrate(&database_url, true, 10)
            .await
            .expect("run migration twice");

        let tables: Vec<String> = query_scalar("SELECT name FROM sqlite_master WHERE type='table'")
            .fetch_all(&pool)
            .await
            .expect("read sqlite table metadata");

        let indexes: Vec<String> = query_scalar(
            "SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .expect("read sqlite index metadata");

        let table_set: BTreeSet<_> = tables.into_iter().collect();
        let index_set: BTreeSet<_> = indexes.into_iter().collect();

        let expected_tables = [
            "users",
            "sessions",
            "api_keys",
            "provider_connections",
            "model_routes",
            "wallets",
            "credit_ledger",
            "usage_events",
            "request_logs",
            "admin_audit_logs",
            "daemon_nodes",
        ];

        let expected_indexes = [
            "idx_api_keys_user_id",
            "idx_api_keys_key_hash",
            "idx_usage_events_user_id_created_at",
            "idx_usage_events_api_key_id_created_at",
            "idx_usage_events_model_created_at",
            "idx_usage_events_provider_id_created_at",
            "idx_usage_events_route_id_created_at",
            "idx_credit_ledger_wallet_created_at",
            "idx_request_logs_user_id_created_at",
            "idx_request_logs_api_key_id_created_at",
            "idx_request_logs_provider_id_created_at",
            "idx_request_logs_route_id_created_at",
            "idx_admin_audit_actor_user_id_created_at",
            "idx_admin_audit_entity_type_created_at",
            "idx_provider_connections_enabled",
            "idx_model_routes_public_model",
            "idx_wallets_owner_user_id",
            "idx_daemon_nodes_host_user_id",
            "idx_daemon_nodes_status",
            "idx_daemon_nodes_last_seen_at",
            "idx_daemon_nodes_token_hash",
        ];

        for expected in expected_tables {
            assert!(table_set.contains(expected), "missing table: {expected}");
        }

        for expected in expected_indexes {
            assert!(index_set.contains(expected), "missing index: {expected}");
        }

        drop(pool);
        fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn ensure_sqlite_parent_directory_handles_nested_paths() {
        let path = std::env::temp_dir()
            .join("mizan-test-parent")
            .join("nested");
        let database_url = format!("sqlite://{}?mode=rwc", path.join("mizan.sqlite3").display());

        ensure_sqlite_parent_directory(&database_url)
            .await
            .expect("ensure directory creation should succeed");

        assert!(path.exists());

        fs::remove_dir_all(std::env::temp_dir().join("mizan-test-parent")).ok();
    }

    #[tokio::test]
    async fn sqlite_in_memory_connection_uses_single_connection() {
        let database_url = "sqlite::memory:".to_string();
        let pool = connect_and_migrate(&database_url, false, 10)
            .await
            .expect("open sqlite memory DB");
        assert!(
            pool.acquire().await.is_ok(),
            "failed to acquire memory connection"
        );
        drop(pool);
    }

    #[test]
    fn in_memory_sqlite_urls_are_detected() {
        assert!(is_sqlite_in_memory("sqlite://:memory:"));
        assert!(is_sqlite_in_memory("sqlite::memory:"));
        assert!(!is_sqlite_in_memory("postgres://localhost/db"));
    }
}
