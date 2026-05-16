const REDACTED: &str = "[REDACTED]";
const SENSITIVE_PREFIXES: [&str; 4] = ["mizan_sk_", "mizan_sess_", "sk-", "Bearer"];

pub fn redact_for_logs(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    let mut redacted = String::with_capacity(value.len());
    let mut segment = String::new();

    for character in value.chars() {
        if character.is_whitespace() {
            redacted.push_str(&redact_segment(&segment));
            segment.clear();
            redacted.push(character);
        } else {
            segment.push(character);
        }
    }

    redacted.push_str(&redact_segment(&segment));
    redacted
}

fn redact_segment(segment: &str) -> String {
    if segment.is_empty() {
        return String::new();
    }

    let mut output = String::with_capacity(segment.len());
    let mut index = 0;
    while index < segment.len() {
        if let Some(prefix) = matching_sensitive_prefix(&segment[index..]) {
            output.push_str(REDACTED);
            index += prefix.len();
            while index < segment.len() {
                let Some(character) = segment[index..].chars().next() else {
                    break;
                };
                if is_secret_boundary(character) {
                    break;
                }
                index += character.len_utf8();
            }
            continue;
        }

        let character = segment[index..]
            .chars()
            .next()
            .expect("index should be on a character boundary");
        output.push(character);
        index += character.len_utf8();
    }

    output
}

fn matching_sensitive_prefix(value: &str) -> Option<&'static str> {
    SENSITIVE_PREFIXES
        .iter()
        .copied()
        .find(|prefix| value.starts_with(prefix))
}

fn is_secret_boundary(character: char) -> bool {
    matches!(character, ',' | ';' | '"' | '\'' | ')' | ']' | '}')
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
    fn redacts_secrets_after_common_delimiters_without_collapsing_whitespace() {
        let input = "key=sk-live-123\nheader=Bearer\tmizan_sk_live_abc token:mizan_sess_456";

        assert_eq!(
            redact_for_logs(input),
            "key=[REDACTED]\nheader=[REDACTED]\t[REDACTED] token:[REDACTED]"
        );
    }

    #[test]
    fn leaves_regular_context_values_visible() {
        let input = "route=mizan-public-model status=200 provider=openai";

        assert_eq!(redact_for_logs(input), input);
    }
}
