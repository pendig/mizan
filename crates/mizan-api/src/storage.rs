use std::path::Path;

use mizan_core::{AppError, AppResult};
use sqlx::{AnyPool, any::AnyPoolOptions, any::install_default_drivers, migrate::Migrator};

static MIGRATOR: Migrator = sqlx::migrate!("./../../migrations");

pub async fn connect_and_migrate(database_url: &str, run_migrations: bool) -> AppResult<AnyPool> {
    ensure_sqlite_parent_directory(database_url)?;

    install_default_drivers();

    let pool = AnyPoolOptions::new()
        .max_connections(10)
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

fn ensure_sqlite_parent_directory(database_url: &str) -> AppResult<()> {
    if !database_url.starts_with("sqlite://") {
        return Ok(());
    }

    let path = database_url.trim_start_matches("sqlite://");
    let path = path.split('?').next().unwrap_or(path);

    if path == ":memory:" || path.is_empty() {
        return Ok(());
    }

    let parent = Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    match parent {
        Some(parent) => {
            std::fs::create_dir_all(parent).map_err(|error| {
                AppError::infrastructure(format!("cannot create sqlite data directory: {error}"))
            })?;
            Ok(())
        }
        None => Ok(()),
    }
}
