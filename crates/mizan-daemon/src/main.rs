use std::{collections::BTreeMap, net::SocketAddr, path::PathBuf, process, str::FromStr};

use clap::{Args, CommandFactory, Parser, Subcommand};
use mizan_core::{AppError, AppResult, init_tracing, redact_for_logs};
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
        Some(Command::Health(args)) => health(args),
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
}

async fn run(args: ConfigArgs) -> AppResult<()> {
    let config = DaemonConfig::load(&args.config)?;
    init_tracing("mizan_daemon=info,mizan_core=info")?;

    info!(
        control_plane_url = %config.control_plane_url,
        daemon_token_path = %redact_for_logs(&config.daemon_token_path),
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

fn health(args: HealthArgs) -> AppResult<()> {
    if let Some(path) = args.config {
        let config = DaemonConfig::load(&path)?;
        println!(
            "ok: daemon config valid; local health bind address {}",
            config.health_addr
        );
    } else {
        println!("ok: mizan-daemon process command is healthy");
    }
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
        let values = parse_simple_toml(raw)?;
        let control_plane_url = required_string(&values, "control_plane_url")?;
        let daemon_token_path = required_string(&values, "daemon_token_path")?;
        let local_provider_url = required_string(&values, "local_provider_url")?;
        let advertised_models = required_list(&values, "advertised_models")?;
        let max_concurrency = optional_u32(&values, "max_concurrency")?.unwrap_or(1);
        let health_addr = optional_string(&values, "health_addr")?
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

fn parse_simple_toml(raw: &str) -> AppResult<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();

    for (line_number, raw_line) in raw.lines().enumerate() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(AppError::invalid_config(
                "config",
                format!("line {} must use key = value", line_number + 1),
            ));
        };
        values.insert(key.trim().to_owned(), value.trim().to_owned());
    }

    Ok(values)
}

fn required_string(values: &BTreeMap<String, String>, key: &'static str) -> AppResult<String> {
    optional_string(values, key)?.ok_or_else(|| AppError::invalid_config(key, "is required"))
}

fn optional_string(
    values: &BTreeMap<String, String>,
    key: &'static str,
) -> AppResult<Option<String>> {
    values
        .get(key)
        .map(|value| parse_quoted_string(value, key))
        .transpose()
}

fn required_list(values: &BTreeMap<String, String>, key: &'static str) -> AppResult<Vec<String>> {
    let Some(value) = values.get(key) else {
        return Err(AppError::invalid_config(key, "is required"));
    };
    let trimmed = value.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Err(AppError::invalid_config(key, "must be a string array"));
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }

    inner
        .split(',')
        .map(|item| parse_quoted_string(item.trim(), key))
        .collect()
}

fn optional_u32(values: &BTreeMap<String, String>, key: &'static str) -> AppResult<Option<u32>> {
    values
        .get(key)
        .map(|value| u32::from_str(value.trim()).map_err(|error| AppError::config(key, error)))
        .transpose()
}

fn parse_quoted_string(value: &str, key: &'static str) -> AppResult<String> {
    let trimmed = value.trim();
    if !trimmed.starts_with('"') || !trimmed.ends_with('"') || trimmed.len() < 2 {
        return Err(AppError::invalid_config(key, "must be a quoted string"));
    }
    Ok(trimmed[1..trimmed.len() - 1].to_owned())
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
