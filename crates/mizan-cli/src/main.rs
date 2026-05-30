use std::io::{self, Write};
use std::process;

use clap::{Args, CommandFactory, Parser, Subcommand};
use futures_util::StreamExt;
use mizan_rtk::{
    ChatProxyConfig, ChatResponse, FilterPolicy, RtkFilterResult,
    chat_completion_request_with_messages, filter_output, passthrough_filter, send_chat_completion,
    send_chat_completion_stream,
};

#[tokio::main]
async fn main() {
    if let Err(error) = execute().await {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

async fn execute() -> Result<(), String> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Filter(args)) => run_filter(args),
        Some(Command::Proxy(args)) => run_proxy(args).await,
        None => {
            Cli::command()
                .print_help()
                .map_err(|error| error.to_string())?;
            println!();
            Ok(())
        }
    }
}

#[derive(Parser)]
#[command(name = "mizan-cli")]
#[command(about = "Utilities for RTK-compatible output filtering and proxying")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Filter(FilterArgs),
    Proxy(ProxyArgs),
}

#[derive(Args)]
struct FilterArgs {
    #[arg(value_name = "TEXT", required = true)]
    input: Vec<String>,
}

#[derive(Args)]
struct ProxyArgs {
    #[arg(long)]
    base_url: String,
    #[arg(long)]
    api_key: String,
    #[arg(long)]
    provider_name: Option<String>,
    #[arg(long)]
    model: String,
    #[arg(long)]
    message: Vec<String>,
    #[arg(long)]
    stream: bool,
    #[arg(long)]
    compact: bool,
    #[arg(long = "max-tokens")]
    max_tokens: Option<u64>,
    #[arg(long)]
    json: bool,
}

fn run_filter(input: FilterArgs) -> Result<(), String> {
    let result = passthrough_filter(input.input.join(" "));
    println!("{}", result.body);
    Ok(())
}

async fn run_proxy(args: ProxyArgs) -> Result<(), String> {
    let request_messages = args
        .message
        .into_iter()
        .map(|content| mizan_rtk::ChatMessage {
            role: "user".to_owned(),
            content,
        })
        .collect::<Vec<_>>();

    if request_messages.is_empty() {
        return Err("--message is required".to_owned());
    }

    let config = ChatProxyConfig {
        api_key: args.api_key,
        base_url: args.base_url,
        provider_name: args.provider_name,
    };

    let request_model = args.model.clone();
    let request = chat_completion_request_with_messages(
        request_model.clone(),
        request_messages,
        args.stream,
        args.max_tokens,
    );

    if args.stream {
        run_streaming_proxy(config, request, request_model, args.compact, args.json).await
    } else {
        let response = send_chat_completion(&config, request)
            .await
            .map_err(|error| error.to_string())?;
        let filtered = if args.compact {
            filter_output(&response.content, &FilterPolicy::default())
        } else {
            passthrough_filter(&response.content)
        };

        if args.json {
            let payload = response_payload(&response, filtered);
            let encoded =
                serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?;
            println!("{encoded}");
        } else {
            println!(
                "{}",
                if args.compact {
                    filtered.body
                } else {
                    response.content
                }
            );
        }

        Ok(())
    }
}

async fn run_streaming_proxy(
    config: ChatProxyConfig,
    request: mizan_rtk::ChatRequest,
    model: String,
    compact_output: bool,
    json_output: bool,
) -> Result<(), String> {
    let mut stream = send_chat_completion_stream(&config, request)
        .await
        .map_err(|error| error.to_string())?;
    let mut content = String::new();
    let mut usage = None;
    let mut emitted = false;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        content.push_str(&chunk.delta);
        if let Some(chunk_usage) = chunk.usage {
            usage = Some(serde_json::to_value(chunk_usage).map_err(|error| error.to_string())?);
        }

        if !json_output && !compact_output {
            print!("{}", chunk.delta);
            io::stdout().flush().map_err(|error| error.to_string())?;
            emitted = true;
        }
    }

    let filtered = if compact_output {
        filter_output(&content, &FilterPolicy::default())
    } else {
        passthrough_filter(&content)
    };

    if json_output {
        let payload = stream_response_payload(&config, model, usage, filtered);
        let encoded = serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?;
        println!("{encoded}");
        return Ok(());
    }

    if compact_output {
        println!("{}", filtered.body);
        return Ok(());
    }

    if emitted {
        println!();
    }
    Ok(())
}

fn response_payload(response: &ChatResponse, filtered: RtkFilterResult) -> serde_json::Value {
    serde_json::json!({
        "provider": response.provider,
        "model": response.model,
        "content": filtered.body,
        "usage": response.usage,
        "filter": {
            "filtered": filtered.filtered,
            "original_chars": filtered.original_chars,
            "output_chars": filtered.output_chars,
        }
    })
}

fn stream_response_payload(
    config: &ChatProxyConfig,
    model: String,
    usage: Option<serde_json::Value>,
    filtered: RtkFilterResult,
) -> serde_json::Value {
    serde_json::json!({
        "provider": config.provider_name.clone().unwrap_or_else(|| provider_fallback_name(&config.base_url)),
        "model": model,
        "content": filtered.body,
        "usage": usage,
        "filter": {
            "filtered": filtered.filtered,
            "original_chars": filtered.original_chars,
            "output_chars": filtered.output_chars,
        }
    })
}

fn provider_fallback_name(base_url: &str) -> String {
    if base_url.to_ascii_lowercase().contains("openai") {
        "openai".to_owned()
    } else {
        "openai-compatible".to_owned()
    }
}
