use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mizan_metering::UsageChargeInput;
use mizan_providers::TokenUsage;
use mizan_wallet::{RoutePrice, calculate_usage_charge};

const LATENCY_BUCKETS_MS: [u64; 7] = [50, 100, 250, 500, 1_000, 2_500, 5_000];

#[derive(Debug, Clone, Default)]
pub struct MetricsRegistry {
    inner: Arc<Mutex<MetricsInner>>,
}

#[derive(Debug, Default)]
struct MetricsInner {
    requests: HashMap<GatewayLabels, u64>,
    prompt_tokens: HashMap<GatewayLabels, u64>,
    completion_tokens: HashMap<GatewayLabels, u64>,
    total_tokens: HashMap<GatewayLabels, u64>,
    credits: HashMap<GatewayLabels, i64>,
    latency: HashMap<GatewayLabels, LatencyHistogram>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GatewayLabels {
    route: String,
    provider: String,
    model: String,
    status: u16,
}

#[derive(Debug, Clone)]
pub struct GatewayObservation {
    pub route: String,
    pub provider: String,
    pub model: String,
    pub status: u16,
    pub usage: TokenUsage,
    pub latency_ms: u64,
    pub route_price: RoutePrice,
}

#[derive(Debug, Clone, Default)]
struct LatencyHistogram {
    buckets: [u64; LATENCY_BUCKETS_MS.len()],
    count: u64,
    sum_ms: u64,
}

impl MetricsRegistry {
    pub fn observe_gateway(&self, observation: GatewayObservation) {
        let labels = GatewayLabels {
            route: normalize_label(observation.route),
            provider: normalize_label(observation.provider),
            model: normalize_label(observation.model),
            status: observation.status,
        };
        let charge = calculate_usage_charge(
            UsageChargeInput {
                prompt_tokens: observation.usage.prompt_tokens,
                completion_tokens: observation.usage.completion_tokens,
            },
            observation.route_price,
        );
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        *inner.requests.entry(labels.clone()).or_default() += 1;
        *inner.prompt_tokens.entry(labels.clone()).or_default() += observation.usage.prompt_tokens;
        *inner.completion_tokens.entry(labels.clone()).or_default() +=
            observation.usage.completion_tokens;
        *inner.total_tokens.entry(labels.clone()).or_default() += observation.usage.total_tokens;
        *inner.credits.entry(labels.clone()).or_default() += charge.total.0.max(0);

        let latency = inner.latency.entry(labels).or_default();
        latency.count += 1;
        latency.sum_ms += observation.latency_ms;
        for (idx, bucket) in LATENCY_BUCKETS_MS.iter().enumerate() {
            if observation.latency_ms <= *bucket {
                latency.buckets[idx] += 1;
            }
        }
    }

    pub fn render_prometheus(&self) -> String {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut output = String::new();

        output.push_str("# HELP mizan_gateway_requests_total Gateway requests by route, provider, model, and status.\n");
        output.push_str("# TYPE mizan_gateway_requests_total counter\n");
        for (labels, value) in &inner.requests {
            output.push_str(&format!(
                "mizan_gateway_requests_total{{{}}} {}\n",
                labels.prometheus(),
                value
            ));
        }

        append_counter(
            &mut output,
            "mizan_gateway_prompt_tokens_total",
            "Prompt tokens observed by the gateway.",
            &inner.prompt_tokens,
        );
        append_counter(
            &mut output,
            "mizan_gateway_completion_tokens_total",
            "Completion tokens observed by the gateway.",
            &inner.completion_tokens,
        );
        append_counter(
            &mut output,
            "mizan_gateway_tokens_total",
            "Total tokens observed by the gateway.",
            &inner.total_tokens,
        );

        output.push_str(
            "# HELP mizan_gateway_credits_charged_total Microcredits charged by the gateway.\n",
        );
        output.push_str("# TYPE mizan_gateway_credits_charged_total counter\n");
        for (labels, value) in &inner.credits {
            output.push_str(&format!(
                "mizan_gateway_credits_charged_total{{{}}} {}\n",
                labels.prometheus(),
                value
            ));
        }

        output.push_str("# HELP mizan_gateway_latency_ms Gateway latency in milliseconds.\n");
        output.push_str("# TYPE mizan_gateway_latency_ms histogram\n");
        for (labels, histogram) in &inner.latency {
            for (idx, bucket) in LATENCY_BUCKETS_MS.iter().enumerate() {
                output.push_str(&format!(
                    "mizan_gateway_latency_ms_bucket{{{},le=\"{}\"}} {}\n",
                    labels.prometheus(),
                    bucket,
                    histogram.buckets[idx]
                ));
            }
            output.push_str(&format!(
                "mizan_gateway_latency_ms_bucket{{{},le=\"+Inf\"}} {}\n",
                labels.prometheus(),
                histogram.count
            ));
            output.push_str(&format!(
                "mizan_gateway_latency_ms_sum{{{}}} {}\n",
                labels.prometheus(),
                histogram.sum_ms
            ));
            output.push_str(&format!(
                "mizan_gateway_latency_ms_count{{{}}} {}\n",
                labels.prometheus(),
                histogram.count
            ));
        }

        output
    }
}

impl GatewayLabels {
    fn prometheus(&self) -> String {
        format!(
            "route=\"{}\",provider=\"{}\",model=\"{}\",status=\"{}\"",
            escape_label(&self.route),
            escape_label(&self.provider),
            escape_label(&self.model),
            self.status
        )
    }
}

fn append_counter(
    output: &mut String,
    name: &str,
    help: &str,
    values: &HashMap<GatewayLabels, u64>,
) {
    output.push_str(&format!("# HELP {name} {help}\n"));
    output.push_str(&format!("# TYPE {name} counter\n"));
    for (labels, value) in values {
        output.push_str(&format!("{name}{{{}}} {}\n", labels.prometheus(), value));
    }
}

fn normalize_label(value: String) -> String {
    let value = value.trim();
    if value.is_empty() {
        "unknown".to_string()
    } else {
        value.to_string()
    }
}

fn escape_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_gateway_request_and_token_metrics() {
        let registry = MetricsRegistry::default();
        registry.observe_gateway(GatewayObservation {
            route: "public-model".to_string(),
            provider: "openai".to_string(),
            model: "gpt-test".to_string(),
            status: 200,
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
                estimated: false,
            },
            latency_ms: 125,
            route_price: RoutePrice {
                input_microcredits_per_1m_tokens: 1,
                output_microcredits_per_1m_tokens: 1,
            },
        });

        let rendered = registry.render_prometheus();

        assert!(rendered.contains("mizan_gateway_requests_total"));
        assert!(rendered.contains("route=\"public-model\""));
        assert!(rendered.contains("mizan_gateway_tokens_total"));
        assert!(rendered.contains("mizan_gateway_latency_ms_bucket"));
    }
}
