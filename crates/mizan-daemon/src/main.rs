use std::{net::SocketAddr, path::PathBuf, process, time::Duration};

use clap::{Args, CommandFactory, Parser, Subcommand};
use mizan_core::{AppError, AppResult, RequestContextBuilder, init_tracing, redact_for_logs};
use mizan_providers::{ChatRequest, ChatResponse, OpenAiCompatibleProvider, ProviderAdapter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    net::TcpStream,
    time::{sleep, timeout},
};
use tracing::{info, warn};
use uuid::Uuid;

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
        Some(Command::Register(args)) => register(args).await,
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
    Register(ConfigArgs),
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
    let token = read_daemon_token(&config)?;
    let heartbeat_url = control_plane_endpoint(&config.control_plane_url, "/daemon/heartbeat");
    let client = daemon_http_client()?;

    info!(
        control_plane_url = %config.control_plane_url,
        daemon_token_path = %config.daemon_token_path,
        local_provider_url = %config.local_provider_url,
        advertised_models = %config.advertised_models.join(","),
        max_concurrency = config.max_concurrency,
        provider_family = %config.provider_family,
        health_addr = %config.health_addr,
        heartbeat_interval_seconds = config.heartbeat_interval_seconds,
        "mizan daemon startup configuration loaded"
    );
    info!("daemon registration is available with `mizan-daemon register --config <path>`");

    loop {
        match send_heartbeat(&client, &heartbeat_url, &token, &config).await {
            Ok(body) => {
                info!(
                    node_id = %body.node_id,
                    status = %body.status,
                    last_seen_at = %body.last_seen_at,
                    "daemon heartbeat accepted by control plane"
                );
            }
            Err(error) => {
                warn!(error = %error, "daemon heartbeat failed");
            }
        }
        if let Err(error) = lease_and_run_one_job(&client, &token, &config).await {
            warn!(error = %error, "daemon dispatch job processing failed");
        }
        sleep(Duration::from_secs(u64::from(
            config.heartbeat_interval_seconds.max(1),
        )))
        .await;
    }
}

async fn lease_and_run_one_job(
    client: &reqwest::Client,
    token: &str,
    config: &DaemonConfig,
) -> AppResult<()> {
    let lease_url = control_plane_endpoint(&config.control_plane_url, "/daemon/jobs/lease");
    let lease_response = client
        .post(&lease_url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| AppError::infrastructure(format!("daemon job lease failed: {error}")))?;

    let status = lease_response.status();
    if !status.is_success() {
        return Err(AppError::infrastructure(format!(
            "daemon job lease rejected by control plane with status {status}"
        )));
    }

    let lease: DispatchJobLeaseEnvelope = lease_response.json().await.map_err(|error| {
        AppError::infrastructure(format!("invalid daemon job lease response: {error}"))
    })?;
    let Some(job) = lease.data else {
        return Ok(());
    };

    let completion = match call_local_provider(config, &job).await {
        Ok(response) => DispatchJobCompleteRequest {
            status: "succeeded".to_owned(),
            response: Some(response),
            error_code: None,
            error_message: None,
        },
        Err(error) => DispatchJobCompleteRequest {
            status: "failed".to_owned(),
            response: None,
            error_code: Some("provider_error".to_owned()),
            error_message: Some(redact_for_logs(error.to_string())),
        },
    };

    let complete_url = control_plane_endpoint(
        &config.control_plane_url,
        &format!("/daemon/jobs/{}/complete", job.id),
    );
    let complete_response = client
        .post(&complete_url)
        .bearer_auth(token)
        .json(&completion)
        .send()
        .await
        .map_err(|error| {
            AppError::infrastructure(format!("daemon job completion submit failed: {error}"))
        })?;

    let status = complete_response.status();
    if !status.is_success() {
        return Err(AppError::infrastructure(format!(
            "daemon job completion rejected by control plane with status {status}"
        )));
    }

    info!(
        job_id = %job.id,
        request_id = %job.request_id,
        model = %job.model,
        "daemon dispatch job completed"
    );
    Ok(())
}

async fn call_local_provider(
    config: &DaemonConfig,
    job: &DispatchJobLeaseResponse,
) -> AppResult<ChatResponse> {
    let request_id = Uuid::parse_str(&job.request_id).map_err(|error| {
        AppError::infrastructure(format!("daemon dispatch request_id is invalid: {error}"))
    })?;
    let context = RequestContextBuilder::default()
        .request_id(request_id)
        .trace_id(request_id)
        .provider("mizan-daemon")
        .route(job.model.clone())
        .model(job.request.model.clone())
        .method("POST")
        .path("/v1/chat/completions")
        .streaming(false)
        .build();
    let provider = OpenAiCompatibleProvider::with_optional_api_key(
        "mizan-daemon",
        config.local_provider_url.clone(),
        config.local_provider_api_key.clone(),
    );

    timeout(
        Duration::from_secs(30),
        provider.chat_completions(&context, job.request.clone()),
    )
    .await
    .map_err(|_| AppError::infrastructure("local provider request timed out after 30 seconds"))?
}

async fn register(args: ConfigArgs) -> AppResult<()> {
    let config = DaemonConfig::load(&args.config)?;
    init_tracing("mizan_daemon=info,mizan_core=info")?;

    let token = read_daemon_token(&config)?;

    let registration_url = control_plane_endpoint(&config.control_plane_url, "/daemon/register");
    let client = daemon_http_client()?;

    let response = client
        .post(&registration_url)
        .bearer_auth(&token)
        .json(&DaemonRegistrationRequest {
            hostname: std::env::var("HOSTNAME")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            public_key: None,
            capabilities: config.capabilities_payload(),
        })
        .send()
        .await
        .map_err(|error| {
            AppError::infrastructure(format!("daemon registration failed: {error}"))
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(AppError::infrastructure(format!(
            "daemon registration rejected by control plane with status {status}"
        )));
    }

    let body: DaemonRegistrationResponse = response.json().await.map_err(|error| {
        AppError::infrastructure(format!("invalid daemon registration response: {error}"))
    })?;

    info!(
        node_id = %body.node_id,
        status = %body.status,
        last_seen_at = %body.last_seen_at,
        "daemon node registered with control plane"
    );
    println!(
        "ok: daemon node {} registered status={} last_seen_at={}",
        body.node_id, body.status, body.last_seen_at
    );
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

async fn send_heartbeat(
    client: &reqwest::Client,
    heartbeat_url: &str,
    token: &str,
    config: &DaemonConfig,
) -> AppResult<DaemonHeartbeatResponse> {
    let response = client
        .post(heartbeat_url)
        .bearer_auth(token)
        .json(&DaemonHeartbeatRequest {
            hostname: std::env::var("HOSTNAME")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            public_key: None,
            capabilities: config.capabilities_payload(),
        })
        .send()
        .await
        .map_err(|error| AppError::infrastructure(format!("daemon heartbeat failed: {error}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(AppError::infrastructure(format!(
            "daemon heartbeat rejected by control plane with status {status}"
        )));
    }

    response.json().await.map_err(|error| {
        AppError::infrastructure(format!("invalid daemon heartbeat response: {error}"))
    })
}

fn read_daemon_token(config: &DaemonConfig) -> AppResult<String> {
    let token = std::fs::read_to_string(&config.daemon_token_path)
        .map_err(|error| AppError::config("daemon_token_path", error))?;
    let token = token.trim();
    if token.is_empty() {
        return Err(AppError::invalid_config(
            "daemon_token_path",
            "daemon token file is empty",
        ));
    }
    Ok(token.to_owned())
}

fn daemon_http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| AppError::infrastructure(format!("daemon http client failed: {error}")))
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

#[derive(Debug, Clone, PartialEq)]
struct DaemonConfig {
    control_plane_url: String,
    daemon_token_path: String,
    local_provider_url: String,
    local_provider_api_key: Option<String>,
    provider_family: String,
    advertised_models: Vec<String>,
    max_concurrency: u32,
    pricing_metadata: Option<Value>,
    region: Option<String>,
    labels: Vec<String>,
    health_addr: SocketAddr,
    heartbeat_interval_seconds: u32,
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
        let local_provider_api_key = raw_config
            .local_provider_api_key
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let provider_family = raw_config
            .provider_family
            .unwrap_or_else(|| "openai-compatible".to_owned())
            .trim()
            .to_ascii_lowercase();
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
        if provider_family.is_empty() {
            return Err(AppError::invalid_config(
                "provider_family",
                "provider_family is required",
            ));
        }
        if max_concurrency == 0 {
            return Err(AppError::invalid_config(
                "max_concurrency",
                "must be greater than zero",
            ));
        }
        let heartbeat_interval_seconds = raw_config.heartbeat_interval_seconds.unwrap_or(30);
        if heartbeat_interval_seconds == 0 {
            return Err(AppError::invalid_config(
                "heartbeat_interval_seconds",
                "must be greater than zero",
            ));
        }

        Ok(Self {
            control_plane_url,
            daemon_token_path,
            local_provider_url,
            local_provider_api_key,
            provider_family,
            advertised_models,
            max_concurrency,
            pricing_metadata: raw_config.pricing_metadata,
            region: raw_config
                .region
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            labels: normalize_string_list(raw_config.labels.unwrap_or_default()),
            health_addr,
            heartbeat_interval_seconds,
        })
    }

    fn capabilities_payload(&self) -> DaemonCapabilityPayload {
        DaemonCapabilityPayload {
            provider_family: self.provider_family.clone(),
            model_ids: self.advertised_models.clone(),
            max_concurrency: self.max_concurrency,
            pricing_metadata: self.pricing_metadata.clone(),
            region: self.region.clone(),
            labels: self.labels.clone(),
            health_status: Some("healthy".to_owned()),
            metadata: Some(serde_json::json!({
                "local_provider_url": self.local_provider_url,
                "health_addr": self.health_addr.to_string(),
            })),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawDaemonConfig {
    control_plane_url: Option<String>,
    daemon_token_path: Option<String>,
    local_provider_url: Option<String>,
    local_provider_api_key: Option<String>,
    provider_family: Option<String>,
    advertised_models: Option<Vec<String>>,
    max_concurrency: Option<u32>,
    pricing_metadata: Option<Value>,
    region: Option<String>,
    labels: Option<Vec<String>>,
    health_addr: Option<String>,
    heartbeat_interval_seconds: Option<u32>,
}

#[derive(Debug, Serialize)]
struct DaemonRegistrationRequest {
    hostname: Option<String>,
    public_key: Option<String>,
    capabilities: DaemonCapabilityPayload,
}

#[derive(Debug, Serialize)]
struct DaemonHeartbeatRequest {
    hostname: Option<String>,
    public_key: Option<String>,
    capabilities: DaemonCapabilityPayload,
}

#[derive(Debug, Serialize)]
struct DaemonCapabilityPayload {
    provider_family: String,
    model_ids: Vec<String>,
    max_concurrency: u32,
    pricing_metadata: Option<Value>,
    region: Option<String>,
    labels: Vec<String>,
    health_status: Option<String>,
    metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct DaemonRegistrationResponse {
    node_id: String,
    status: String,
    last_seen_at: String,
}

#[derive(Debug, Deserialize)]
struct DaemonHeartbeatResponse {
    node_id: String,
    status: String,
    last_seen_at: String,
}

#[derive(Debug, Deserialize)]
struct DispatchJobLeaseEnvelope {
    data: Option<DispatchJobLeaseResponse>,
}

#[derive(Debug, Deserialize)]
struct DispatchJobLeaseResponse {
    id: String,
    request_id: String,
    model: String,
    request: ChatRequest,
}

#[derive(Debug, Serialize)]
struct DispatchJobCompleteRequest {
    status: String,
    response: Option<ChatResponse>,
    error_code: Option<String>,
    error_message: Option<String>,
}

fn required_field<T>(value: Option<T>, key: &'static str) -> AppResult<T> {
    value.ok_or_else(|| AppError::invalid_config(key, "is required"))
}

fn control_plane_endpoint(control_plane_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        control_plane_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty()
            || normalized
                .iter()
                .any(|candidate: &String| candidate.as_str() == value)
        {
            continue;
        }
        normalized.push(value.to_owned());
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_core::redact_for_logs;
    use mizan_providers::ChatMessage;

    const VALID_CONFIG: &str = r#"
control_plane_url = "https://mizan.example.test"
daemon_token_path = "/run/secrets/mizan-daemon-token"
local_provider_url = "http://127.0.0.1:11434/v1"
provider_family = "openai-compatible"
advertised_models = ["llama3.1", "qwen2.5-coder"]
max_concurrency = 4
region = "local"
labels = ["gpu", "lab"]
health_addr = "127.0.0.1:19180"
heartbeat_interval_seconds = 15
"#;

    #[test]
    fn parses_valid_config() {
        let config = DaemonConfig::parse(VALID_CONFIG).expect("config should parse");

        assert_eq!(config.control_plane_url, "https://mizan.example.test");
        assert_eq!(config.local_provider_api_key, None);
        assert_eq!(
            config.advertised_models,
            vec!["llama3.1".to_owned(), "qwen2.5-coder".to_owned()]
        );
        assert_eq!(config.provider_family, "openai-compatible");
        assert_eq!(config.max_concurrency, 4);
        assert_eq!(config.region.as_deref(), Some("local"));
        assert_eq!(config.labels, vec!["gpu".to_owned(), "lab".to_owned()]);
        assert_eq!(
            config.health_addr,
            "127.0.0.1:19180".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(config.heartbeat_interval_seconds, 15);
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
    fn parses_optional_local_provider_api_key() {
        let raw = VALID_CONFIG.replace(
            "local_provider_url = \"http://127.0.0.1:11434/v1\"",
            "local_provider_url = \"http://127.0.0.1:11434/v1\"\n\
             local_provider_api_key = \" local-secret \"",
        );
        let config = DaemonConfig::parse(&raw).expect("config should parse");

        assert_eq!(
            config.local_provider_api_key.as_deref(),
            Some("local-secret")
        );
    }

    #[test]
    fn builds_capability_payload_from_config() {
        let config = DaemonConfig::parse(VALID_CONFIG).expect("config should parse");

        let payload = config.capabilities_payload();

        assert_eq!(payload.provider_family, "openai-compatible");
        assert_eq!(
            payload.model_ids,
            vec!["llama3.1".to_owned(), "qwen2.5-coder".to_owned()]
        );
        assert_eq!(payload.max_concurrency, 4);
        assert_eq!(payload.region.as_deref(), Some("local"));
        assert_eq!(payload.health_status.as_deref(), Some("healthy"));
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
    fn rejects_zero_heartbeat_interval() {
        let raw = VALID_CONFIG.replace(
            "heartbeat_interval_seconds = 15",
            "heartbeat_interval_seconds = 0",
        );
        let error = DaemonConfig::parse(&raw).expect_err("config should fail");

        assert!(error.to_string().contains("heartbeat_interval_seconds"));
    }

    #[test]
    fn builds_registration_endpoint_without_double_slashes() {
        assert_eq!(
            control_plane_endpoint("https://mizan.example.test/", "/daemon/register"),
            "https://mizan.example.test/daemon/register"
        );
    }

    #[test]
    fn redacts_secret_material_for_logs() {
        let input = "daemon_token=mizan_sk_daemon_123 bearer=Bearer abc";

        assert_eq!(
            redact_for_logs(input),
            "daemon_token=[REDACTED] bearer=[REDACTED] abc"
        );
    }

    #[tokio::test]
    async fn local_openai_compatible_provider_is_called_and_normalized() {
        let request_id = Uuid::now_v7();
        let server = MockOpenAiServer::spawn(request_id);
        let raw_config = VALID_CONFIG.replace(
            "local_provider_url = \"http://127.0.0.1:11434/v1\"",
            &format!("local_provider_url = \"{}\"", server.base_url),
        );
        let config = DaemonConfig::parse(&raw_config).expect("config should parse");
        let job = DispatchJobLeaseResponse {
            id: Uuid::now_v7().to_string(),
            request_id: request_id.to_string(),
            model: "public-llama".to_owned(),
            request: ChatRequest {
                model: "llama3.1".to_owned(),
                messages: vec![ChatMessage {
                    role: "user".to_owned(),
                    content: "hello".to_owned(),
                }],
                stream: false,
                max_tokens: Some(16),
            },
        };

        let response = call_local_provider(&config, &job)
            .await
            .expect("local provider should respond");

        server.assert_ok();
        assert_eq!(response.provider, "mizan-daemon");
        assert_eq!(response.model, "llama3.1");
        assert_eq!(response.content, "hello from mock");
        let usage = response.usage.expect("usage should be propagated");
        assert_eq!(usage.prompt_tokens, 3);
        assert_eq!(usage.completion_tokens, 4);
        assert_eq!(usage.total_tokens, 7);
        assert!(!usage.estimated);
    }

    struct MockOpenAiServer {
        base_url: String,
        result_rx: std::sync::mpsc::Receiver<Result<(), String>>,
    }

    impl MockOpenAiServer {
        fn spawn(expected_request_id: Uuid) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let addr = listener.local_addr().expect("mock server addr");
            let (result_tx, result_rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = handle_mock_openai_request(listener, expected_request_id);
                let _ = result_tx.send(result);
            });

            Self {
                base_url: format!("http://{addr}"),
                result_rx,
            }
        }

        fn assert_ok(self) {
            let result = self
                .result_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("mock server should finish");
            result.expect("mock server request should be valid");
        }
    }

    fn handle_mock_openai_request(
        listener: std::net::TcpListener,
        expected_request_id: Uuid,
    ) -> Result<(), String> {
        use std::io::{Read, Write};

        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        let header_end = loop {
            let read = stream
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("connection closed before headers".to_owned());
            }
            request.extend_from_slice(&buffer[..read]);
            if let Some(position) = find_header_end(&request) {
                break position;
            }
        };

        let headers = String::from_utf8_lossy(&request[..header_end]).to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .ok_or_else(|| "missing content-length".to_owned())?;
        while request.len() < header_end + 4 + content_length {
            let read = stream
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("connection closed before body".to_owned());
            }
            request.extend_from_slice(&buffer[..read]);
        }

        assert!(headers.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(headers.lines().any(|line| {
            line.eq_ignore_ascii_case(&format!("x-request-id: {expected_request_id}"))
        }));
        assert!(
            !headers
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "local provider auth should be omitted when no key is configured"
        );

        let body_start = header_end + 4;
        let payload: serde_json::Value =
            serde_json::from_slice(&request[body_start..body_start + content_length])
                .map_err(|error| error.to_string())?;
        assert_eq!(payload["model"], "llama3.1");
        assert_eq!(payload["stream"], false);
        assert_eq!(payload["max_tokens"], 16);

        let body = serde_json::json!({
            "id": "chatcmpl-mock",
            "object": "chat.completion",
            "model": "llama3.1",
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": "hello from mock"},
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 3,
                "completion_tokens": 4,
                "total_tokens": 7
            }
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .map_err(|error| error.to_string())?;

        Ok(())
    }

    fn find_header_end(bytes: &[u8]) -> Option<usize> {
        bytes.windows(4).position(|window| window == b"\r\n\r\n")
    }
}
