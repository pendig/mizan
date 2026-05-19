use std::env;
use std::process;

use mizan_rtk::{
    chat_completion_request_with_messages, filter_output, passthrough_filter, ChatProxyConfig,
    ChatResponse, FilterPolicy, RtkFilterResult,
};

#[tokio::main]
async fn main() {
    match parse_command() {
        Ok(Command::Help) => print_usage(),
        Ok(Command::Filter { input }) => {
            let result = passthrough_filter(input);
            println!("{}", result.body);
        }
        Ok(Command::Proxy {
            config,
            model,
            messages,
            stream,
            compact_output,
            max_tokens,
            json_output,
        }) => {
            let request = chat_completion_request_with_messages(
                model,
                messages,
                stream,
                max_tokens,
            );

            let config = ChatProxyConfig {
                api_key: config.api_key,
                base_url: config.base_url,
                provider_name: config.provider_name,
            };
            match response_for(&config, request).await {
                Ok(response) => {
                    let policy = FilterPolicy::default();

                    let filtered = if compact_output {
                        filter_output(&response.content, &policy)
                    } else {
                        passthrough_filter(&response.content)
                    };
                    if json_output {
                        match serde_json::to_string_pretty(&response_payload(&response, filtered)) {
                            Ok(json) => println!("{json}"),
                            Err(error) => {
                                eprintln!("failed to encode json output: {error}");
                                process::exit(1);
                            }
                        }
                    } else if compact_output {
                        if !filtered.body.is_empty() {
                            println!("{}", filtered.body);
                        }
                    } else {
                        println!("{}", response.content);
                    }
                }
                Err(error) => {
                    eprintln!("proxy error: {error}");
                    process::exit(1);
                }
            };
        }
        Err(error) => {
            eprintln!("error: {error}");
            print_usage();
            process::exit(1);
        }
    }
}

async fn response_for(
    config: &ChatProxyConfig,
    request: mizan_rtk::ChatRequest,
) -> Result<mizan_rtk::ChatResponse, String> {
    mizan_rtk::send_chat_completion(config, request)
        .await
        .map_err(|error| error.to_string())
}

fn response_payload(
    response: &ChatResponse,
    filtered: RtkFilterResult,
) -> serde_json::Value {
    serde_json::json!({
        "provider": response.provider,
        "model": response.model,
        "content": filtered.body,
        "usage": response.usage,
        "filter": {
            "filtered": filtered.filtered,
            "original_chars": filtered.original_chars,
            "output_chars": filtered.output_chars
        }
    })
}

struct ProxyConfigArgs {
    base_url: String,
    api_key: String,
    provider_name: Option<String>,
}

enum Command {
    Help,
    Filter {
        input: String,
    },
    Proxy {
        config: ProxyConfigArgs,
        model: String,
        messages: Vec<mizan_rtk::ChatMessage>,
        stream: bool,
        compact_output: bool,
        max_tokens: Option<u64>,
        json_output: bool,
    },
}

fn parse_command() -> Result<Command, String> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        return Ok(Command::Help);
    }

    let command = args.remove(0);
    match command.as_str() {
        "help" | "--help" | "-h" => Ok(Command::Help),
        "filter" => {
            if args.is_empty() {
                return Err("filter requires input text".to_owned());
            }
            Ok(Command::Filter {
                input: args.join(" "),
            })
        }
        "proxy" => parse_proxy(&mut args),
        _ => Err(format!("unknown command: {command}")),
    }
}

fn parse_proxy(args: &mut Vec<String>) -> Result<Command, String> {
    let mut base_url = None;
    let mut api_key = None;
    let mut provider_name = None;
    let mut model = None;
    let mut messages = Vec::new();
    let mut stream = false;
    let mut max_tokens = None;
    let mut compact_output = false;
    let mut json_output = false;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--base-url" => {
                i += 1;
                base_url = Some(
                    args.get(i)
                        .cloned()
                        .ok_or("--base-url requires value")?,
                );
            }
            "--api-key" => {
                i += 1;
                api_key = Some(
                    args.get(i)
                        .cloned()
                        .ok_or("--api-key requires value")?,
                );
            }
            "--provider-name" => {
                i += 1;
                provider_name = Some(
                    args.get(i)
                        .cloned()
                        .ok_or("--provider-name requires value")?,
                );
            }
            "--model" => {
                i += 1;
                model = Some(args.get(i).cloned().ok_or("--model requires value")?);
            }
            "--message" => {
                i += 1;
                let value = args
                    .get(i)
                    .cloned()
                    .ok_or("--message requires value")?;
                messages.push(mizan_rtk::ChatMessage {
                    role: "user".to_owned(),
                    content: value,
                });
            }
            "--stream" => {
                stream = true;
            }
            "--json" => {
                json_output = true;
            }
            "--compact" => {
                compact_output = true;
            }
            "--max-tokens" => {
                i += 1;
                let raw_value = args.get(i).ok_or("--max-tokens requires value".to_owned())?;
                max_tokens = Some(
                    raw_value
                        .parse::<u64>()
                        .map_err(|_| format!("invalid --max-tokens value: {raw_value}"))?,
                );
            }
            value => {
                if value.starts_with('-') {
                    return Err(format!("unknown argument: {value}"));
                }

                messages.push(mizan_rtk::ChatMessage {
                    role: "user".to_owned(),
                    content: value.to_owned(),
                });
            }
        }
        i += 1;
    }

    if model.is_none() {
        return Err("--model is required".to_owned());
    }
    if messages.is_empty() {
        return Err("--message is required".to_owned());
    }

    Ok(Command::Proxy {
        config: ProxyConfigArgs {
            base_url: base_url.ok_or("--base-url is required".to_owned())?,
            api_key: api_key.ok_or("--api-key is required".to_owned())?,
            provider_name,
        },
        model: model.expect("model is required"),
        messages,
        stream,
        compact_output,
        max_tokens,
        json_output,
    })
}

fn print_usage() {
    println!("mizan-cli");
    println!("Usage:");
    println!("  mizan-cli filter <text>");
    println!("  mizan-cli proxy --base-url <url> --api-key <key> --model <model> --message <text> [--provider-name <name>] [--stream] [--max-tokens N] [--compact] [--json]");
}
