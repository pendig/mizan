use std::{net::SocketAddr, path::PathBuf, process, time::Duration};

use clap::{Args, CommandFactory, Parser, Subcommand};
use mizan_core::{AppError, AppResult, init_tracing, redact_for_logs};
use serde::Deserialize;
use tokio::{net::TcpStream, time::timeout};
use tracing::info;

#[tokio::main]
async fn main() {
    if let Err(error) = execute().await {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

async fn execute() -> AppResult<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Run(args)) => run(args).await,
        Some(Command::ConfigCheck(args)) => config_check(args),
        Some(Command::Health(args)) => health(args).await,
        None => {
            Cli::command().print_help()?;
            println!();
            Ok(())
        }
    }
}

#[derive(Parser)]
#[command(name = "mizan-daemon")]
#[command(about = "Self-hosted Mizan daemon for distributed proxy capacity")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Run(ConfigArgs),
    ConfigCheck(ConfigArgs),
    Health(HealthArgs),
}

#[derive(Args)]
struct ConfigArgs {
    #[arg(long, value_name = "PATH")]
    config: PathBuf,
}

#[derive(Args)]
struct HealthArgs {
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    #[arg(long, default_value_t = 1000, value_name = "MILLISECONDS")]
    timeout_ms: u64,
}

async fn run(args: ConfigArgs) -> AppResult<()> {
    let config = DaemonConfig::load(&args.config)?;
    init_tracing("mizan_daemon=info,mizan_core=info")?;

    info!(
        control_plane_url = %config.control_plane_url,
        daemon_token_path = %config.daemon_token_path,
        local_provider_url = %config.local_provider_url,
        advertised_models = %config.advertised_models.join(","),
        max_concurrency = config.max_concurrency,
        health_addr = %config.health_addr,
        "mizan daemon startup configuration loaded"
    );
    info!("daemon registration is prepared; node registration lands in the next milestone task");

    Ok(())
}

fn config_check(args: ConfigArgs) -> AppResult<()> {
    let config = DaemonConfig::load(&args.config)?;
    println!(
        "ok: control_plane_url={} local_provider_url={} advertised_models={} max_concurrency={} health_addr={}",
        config.control_plane_url,
        config.local_provider_url,
        config.advertised_models.join(","),
        config.max_concurrency,
        config.health_addr
    );
    Ok(())
}

async fn health(args: HealthArgs) -> AppResult<()> {
    if let Some(path) = args.config {
        let config = DaemonConfig::load(&path)?;
        probe_health_addr(config.health_addr, args.timeout_ms).await?;
        println!(
            "ok: daemon health endpoint reachable at {}",
            config.health_addr
        );
    } else {
        println!("ok: mizan-daemon process command is healthy");
    }
    Ok(())
}

async fn probe_health_addr(addr: SocketAddr, timeout_ms: u64) -> AppResult<()> {
    let timeout_duration = Duration::from_millis(timeout_ms.max(1));
    timeout(timeout_duration, TcpStream::connect(addr))
        .await
        .map_err(|_| AppError::infrastructure(format!("health probe timed out for {addr}")))?
        .map_err(|error| {
            AppError::infrastructure(format!("health probe failed for {addr}: {error}"))
        })?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonConfig {
    control_plane_url: String,
    daemon_token_path: String,
    local_provider_url: String,
    advertised_models: Vec<String>,
    max_concurrency: u32,
    health_addr: SocketAddr,
}

impl DaemonConfig {
    fn load(path: &PathBuf) -> AppResult<Self> {
        let raw = std::fs::read_to_string(path)?;
        Self::parse(&raw)
    }

    fn parse(raw: &str) -> AppResult<Self> {
        let raw_config: RawDaemonConfig =
            toml::from_str(raw).map_err(|error| AppError::config("config", error))?;
        let control_plane_url = required_field(raw_config.control_plane_url, "control_plane_url")?;
        let daemon_token_path = required_field(raw_config.daemon_token_path, "daemon_token_path")?;
        let local_provider_url =
            required_field(raw_config.local_provider_url, "local_provider_url")?;
        let advertised_models = required_field(raw_config.advertised_models, "advertised_models")?;
        let max_concurrency = raw_config.max_concurrency.unwrap_or(1);
        let health_addr = raw_config
            .health_addr
            .unwrap_or_else(|| "127.0.0.1:19180".to_owned())
            .parse::<SocketAddr>()
            .map_err(|error| AppError::config("health_addr", error))?;

        if advertised_models.is_empty() {
            return Err(AppError::invalid_config(
                "advertised_models",
                "at least one model is required",
            ));
        }
        if max_concurrency == 0 {
            return Err(AppError::invalid_config(
                "max_concurrency",
                "must be greater than zero",
            ));
        }

        Ok(Self {
            control_plane_url,
            daemon_token_path,
            local_provider_url,
            advertised_models,
            max_concurrency,
            health_addr,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawDaemonConfig {
    control_plane_url: Option<String>,
    daemon_token_path: Option<String>,
    local_provider_url: Option<String>,
    advertised_models: Option<Vec<String>>,
    max_concurrency: Option<u32>,
    health_addr: Option<String>,
}

fn required_field<T>(value: Option<T>, key: &'static str) -> AppResult<T> {
    value.ok_or_else(|| AppError::invalid_config(key, "is required"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_CONFIG: &str = r#"
control_plane_url = "https://mizan.example.test"
daemon_token_path = "/run/secrets/mizan-daemon-token"
local_provider_url = "http://127.0.0.1:11434/v1"
advertised_models = ["llama3.1", "qwen2.5-coder"]
max_concurrency = 4
health_addr = "127.0.0.1:19180"
"#;

    #[test]
    fn parses_valid_config() {
        let config = DaemonConfig::parse(VALID_CONFIG).expect("config should parse");

        assert_eq!(config.control_plane_url, "https://mizan.example.test");
        assert_eq!(
            config.advertised_models,
            vec!["llama3.1".to_owned(), "qwen2.5-coder".to_owned()]
        );
        assert_eq!(config.max_concurrency, 4);
        assert_eq!(
            config.health_addr,
            "127.0.0.1:19180".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn parses_toml_strings_with_comment_and_comma_characters() {
        let raw = VALID_CONFIG
            .replace(
                "/run/secrets/mizan-daemon-token",
                "/run/secrets/mizan#daemon-token",
            )
            .replace(
                r#""llama3.1", "qwen2.5-coder""#,
                r#""llama3.1", "qwen2.5, coder""#,
            );

        let config = DaemonConfig::parse(&raw).expect("config should parse");

        assert_eq!(config.daemon_token_path, "/run/secrets/mizan#daemon-token");
        assert_eq!(
            config.advertised_models,
            vec!["llama3.1".to_owned(), "qwen2.5, coder".to_owned()]
        );
    }

    #[test]
    fn rejects_missing_required_fields() {
        let error = DaemonConfig::parse("control_plane_url = \"https://mizan.example.test\"")
            .expect_err("config should fail");

        assert!(error.to_string().contains("daemon_token_path"));
    }

    #[test]
    fn rejects_zero_concurrency() {
        let raw = VALID_CONFIG.replace("max_concurrency = 4", "max_concurrency = 0");
        let error = DaemonConfig::parse(&raw).expect_err("config should fail");

        assert!(error.to_string().contains("max_concurrency"));
    }

    #[test]
    fn redacts_secret_material_for_logs() {
        let input = "daemon_token=mizan_sk_daemon_123 bearer=Bearer abc";

        assert_eq!(
            redact_for_logs(input),
            "daemon_token=[REDACTED] bearer=[REDACTED] abc"
        );
    }
}
