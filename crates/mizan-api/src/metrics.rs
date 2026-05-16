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
    stats: HashMap<GatewayLabels, MetricSet>,
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
struct MetricSet {
    requests: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    credits: i64,
    latency: LatencyHistogram,
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
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let metrics = inner.stats.entry(labels).or_default();

        metrics.requests += 1;
        metrics.prompt_tokens += observation.usage.prompt_tokens;
        metrics.completion_tokens += observation.usage.completion_tokens;
        metrics.total_tokens += observation.usage.total_tokens;
        if (200..=299).contains(&observation.status) {
            let charge = calculate_usage_charge(
                UsageChargeInput {
                    prompt_tokens: observation.usage.prompt_tokens,
                    completion_tokens: observation.usage.completion_tokens,
                },
                observation.route_price,
            );
            metrics.credits += charge.total.0.max(0);
        }

        metrics.latency.count += 1;
        metrics.latency.sum_ms += observation.latency_ms;
        for (idx, bucket) in LATENCY_BUCKETS_MS.iter().enumerate() {
            if observation.latency_ms <= *bucket {
                metrics.latency.buckets[idx] += 1;
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
        for (labels, metrics) in &inner.stats {
            output.push_str(&format!(
                "mizan_gateway_requests_total{{{}}} {}\n",
                labels.prometheus(),
                metrics.requests
            ));
        }

        append_counter(
            &mut output,
            "mizan_gateway_prompt_tokens_total",
            "Prompt tokens observed by the gateway.",
            &inner.stats,
            |metrics| metrics.prompt_tokens,
        );
        append_counter(
            &mut output,
            "mizan_gateway_completion_tokens_total",
            "Completion tokens observed by the gateway.",
            &inner.stats,
            |metrics| metrics.completion_tokens,
        );
        append_counter(
            &mut output,
            "mizan_gateway_tokens_total",
            "Total tokens observed by the gateway.",
            &inner.stats,
            |metrics| metrics.total_tokens,
        );

        output.push_str(
            "# HELP mizan_gateway_credits_charged_total Microcredits charged by the gateway.\n",
        );
        output.push_str("# TYPE mizan_gateway_credits_charged_total counter\n");
        for (labels, metrics) in &inner.stats {
            output.push_str(&format!(
                "mizan_gateway_credits_charged_total{{{}}} {}\n",
                labels.prometheus(),
                metrics.credits
            ));
        }

        output.push_str("# HELP mizan_gateway_latency_ms Gateway latency in milliseconds.\n");
        output.push_str("# TYPE mizan_gateway_latency_ms histogram\n");
        for (labels, metrics) in &inner.stats {
            for (idx, bucket) in LATENCY_BUCKETS_MS.iter().enumerate() {
                output.push_str(&format!(
                    "mizan_gateway_latency_ms_bucket{{{},le=\"{}\"}} {}\n",
                    labels.prometheus(),
                    bucket,
                    metrics.latency.buckets[idx]
                ));
            }
            output.push_str(&format!(
                "mizan_gateway_latency_ms_bucket{{{},le=\"+Inf\"}} {}\n",
                labels.prometheus(),
                metrics.latency.count
            ));
            output.push_str(&format!(
                "mizan_gateway_latency_ms_sum{{{}}} {}\n",
                labels.prometheus(),
                metrics.latency.sum_ms
            ));
            output.push_str(&format!(
                "mizan_gateway_latency_ms_count{{{}}} {}\n",
                labels.prometheus(),
                metrics.latency.count
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
    stats: &HashMap<GatewayLabels, MetricSet>,
    value: impl Fn(&MetricSet) -> u64,
) {
    output.push_str(&format!("# HELP {name} {help}\n"));
    output.push_str(&format!("# TYPE {name} counter\n"));
    for (labels, metrics) in stats {
        output.push_str(&format!(
            "{name}{{{}}} {}\n",
            labels.prometheus(),
            value(metrics)
        ));
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
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
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

    #[test]
    fn credits_are_counted_only_for_successful_charges() {
        let registry = MetricsRegistry::default();
        let usage = TokenUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
            estimated: false,
        };
        let route_price = RoutePrice {
            input_microcredits_per_1m_tokens: 1,
            output_microcredits_per_1m_tokens: 1,
        };

        registry.observe_gateway(GatewayObservation {
            route: "public-model".to_string(),
            provider: "openai".to_string(),
            model: "gpt-test".to_string(),
            status: 200,
            usage,
            latency_ms: 10,
            route_price,
        });
        registry.observe_gateway(GatewayObservation {
            route: "public-model".to_string(),
            provider: "openai".to_string(),
            model: "gpt-test".to_string(),
            status: 429,
            usage,
            latency_ms: 10,
            route_price,
        });

        let rendered = registry.render_prometheus();

        assert!(rendered.contains(
            "mizan_gateway_credits_charged_total{route=\"public-model\",provider=\"openai\",model=\"gpt-test\",status=\"200\"} 2"
        ));
        assert!(rendered.contains(
            "mizan_gateway_credits_charged_total{route=\"public-model\",provider=\"openai\",model=\"gpt-test\",status=\"429\"} 0"
        ));
    }

    #[test]
    fn prometheus_labels_escape_newlines_quotes_and_backslashes() {
        let registry = MetricsRegistry::default();
        registry.observe_gateway(GatewayObservation {
            route: "bad\nroute".to_string(),
            provider: "provider\"quote".to_string(),
            model: "model\\slash".to_string(),
            status: 200,
            usage: TokenUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                estimated: true,
            },
            latency_ms: 1,
            route_price: RoutePrice {
                input_microcredits_per_1m_tokens: 0,
                output_microcredits_per_1m_tokens: 0,
            },
        });

        let rendered = registry.render_prometheus();

        assert!(rendered.contains("route=\"bad\\nroute\""));
        assert!(rendered.contains("provider=\"provider\\\"quote\""));
        assert!(rendered.contains("model=\"model\\\\slash\""));
    }
}
