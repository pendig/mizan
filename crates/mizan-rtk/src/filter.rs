use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterPolicy {
    pub max_output_chars: usize,
    pub retained_head_chars: usize,
    pub retained_tail_chars: usize,
}

impl Default for FilterPolicy {
    fn default() -> Self {
        Self {
            max_output_chars: 4_000,
            retained_head_chars: 1_500,
            retained_tail_chars: 800,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtkFilterResult {
    pub original_chars: usize,
    pub output_chars: usize,
    pub body: String,
    pub filtered: bool,
}

pub fn passthrough_filter(output: impl Into<String>) -> RtkFilterResult {
    let body = output.into();
    let body_len = body.chars().count();

    RtkFilterResult {
        original_chars: body_len,
        output_chars: body_len,
        filtered: false,
        body,
    }
}

pub fn filter_output(output: impl Into<String>, policy: &FilterPolicy) -> RtkFilterResult {
    let body = output.into();
    let chars: Vec<char> = body.chars().collect();
    let body_len = chars.len();

    if body_len <= policy.max_output_chars {
        return RtkFilterResult {
            original_chars: body_len,
            output_chars: body_len,
            filtered: false,
            body,
        };
    }

    let mut retained_head_chars = policy.retained_head_chars.min(body_len);
    let mut retained_tail_chars = policy
        .retained_tail_chars
        .min(body_len.saturating_sub(retained_head_chars));

    while retained_head_chars + retained_tail_chars > policy.max_output_chars {
        if retained_tail_chars > 0 {
            retained_tail_chars -= 1;
            continue;
        }
        if retained_head_chars > 0 {
            retained_head_chars -= 1;
        } else {
            break;
        }
    }

    loop {
        let marker = truncation_marker(retained_head_chars, retained_tail_chars);
        let output_len = retained_head_chars + retained_tail_chars + marker.chars().count();

        if output_len <= policy.max_output_chars {
            break;
        }

        if retained_tail_chars > 0 {
            retained_tail_chars -= 1;
            continue;
        }
        if retained_head_chars > 0 {
            retained_head_chars -= 1;
            continue;
        }
        break;
    }

    let marker = truncation_marker(retained_head_chars, retained_tail_chars);
    let truncated = truncate_middle(&chars, retained_head_chars, retained_tail_chars);
    let body = format!("{}{}{}", truncated.0, marker, truncated.1);
    let output_len = body.chars().count();

    let (body, output_len) = if output_len > policy.max_output_chars {
        let truncated_body: String = body.chars().take(policy.max_output_chars).collect();
        let output_chars = truncated_body.chars().count();

        (truncated_body, output_chars)
    } else {
        (body, output_len)
    };

    RtkFilterResult {
        original_chars: body_len,
        output_chars: output_len,
        filtered: true,
        body,
    }
}

fn truncation_marker(head_chars: usize, tail_chars: usize) -> String {
    format!(
        "\n\n[output truncated by mizan-rtk: {} chars retained from head and {} from tail]\n",
        head_chars, tail_chars,
    )
}

fn truncate_middle(chars: &[char], head_chars: usize, tail_chars: usize) -> (String, String) {
    let head_chars = head_chars.min(chars.len());
    let tail_chars = tail_chars.min(chars.len().saturating_sub(head_chars));
    let head = chars[..head_chars].iter().collect::<String>();
    let tail = chars[chars.len().saturating_sub(tail_chars)..]
        .iter()
        .collect::<String>();

    (head, tail)
}

#[cfg(test)]
mod tests {
    use super::{FilterPolicy, filter_output};

    #[test]
    fn filters_long_output_with_marker() {
        let input = "A".repeat(5_000);
        let policy = FilterPolicy {
            max_output_chars: 100,
            retained_head_chars: 20,
            retained_tail_chars: 20,
        };
        let result = filter_output(input, &policy);

        assert!(result.filtered);
        assert!(result.body.contains("output truncated by mizan-rtk"));
        assert!(result.output_chars < result.original_chars);
        assert!(result.output_chars <= 100);
    }

    #[test]
    fn filters_with_tight_output_limit() {
        let input = "abcdefghijklmnopqrstuvwxyz".repeat(100);
        let policy = FilterPolicy {
            max_output_chars: 10,
            retained_head_chars: 80,
            retained_tail_chars: 80,
        };
        let result = filter_output(input, &policy);

        assert!(result.filtered);
        assert!(result.output_chars <= policy.max_output_chars);
        assert_eq!(result.output_chars, result.body.chars().count());
    }
}
