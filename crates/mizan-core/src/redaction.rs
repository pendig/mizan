const REDACTED: &str = "[REDACTED]";
const SENSITIVE_PREFIXES: [&str; 4] = ["mizan_sk_", "mizan_sess_", "sk-", "Bearer"];

pub fn redact_for_logs(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .split_whitespace()
        .map(redact_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_token(token: &str) -> String {
    if SENSITIVE_PREFIXES
        .iter()
        .any(|prefix| token.starts_with(prefix))
    {
        return REDACTED.to_string();
    }

    token.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_secret_prefixes() {
        let input = "Bearer mizan_sk_live_abc sk-live-123 mizan_sess_456";

        assert_eq!(
            redact_for_logs(input),
            "[REDACTED] [REDACTED] [REDACTED] [REDACTED]"
        );
    }

    #[test]
    fn leaves_regular_context_values_visible() {
        let input = "route=mizan-public-model status=200 provider=openai";

        assert_eq!(redact_for_logs(input), input);
    }
}
