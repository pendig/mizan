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

pub fn filter_output(
    output: impl Into<String>,
    policy: &FilterPolicy,
) -> RtkFilterResult {
    let body = output.into();
    let body_len = body.chars().count();

    if body_len <= policy.max_output_chars {
        return RtkFilterResult {
            original_chars: body_len,
            output_chars: body_len,
            filtered: false,
            body,
        };
    }

    let truncated = truncate_middle(
        &body,
        policy.retained_head_chars,
        policy.retained_tail_chars,
    );
    let marker = format!(
        "\n\n[output truncated by mizan-rtk: {} chars retained from head and {} from tail]\n",
        policy.retained_head_chars.min(body_len),
        policy.retained_tail_chars.min(body_len.saturating_sub(policy.retained_head_chars)),
    );
    let body = format!("{}{}{}", truncated.0, marker, truncated.1);
    let output_len = body.chars().count();

    RtkFilterResult {
        original_chars: body_len,
        output_chars: output_len,
        filtered: true,
        body,
    }
}

fn truncate_middle(input: &str, head_chars: usize, tail_chars: usize) -> (String, String) {
    let mut head = String::with_capacity(head_chars * 4);
    let mut tail = String::with_capacity(tail_chars * 4);

    let head_chars = head_chars.min(input.chars().count());
    let tail_chars = tail_chars.min(input.chars().count().saturating_sub(head_chars));

    for ch in input.chars().take(head_chars) {
        head.push(ch);
    }

    for ch in input.chars().rev().take(tail_chars).collect::<Vec<_>>().into_iter().rev() {
        tail.push(ch);
    }

    (head, tail)
}

#[cfg(test)]
mod tests {
    use super::{filter_output, FilterPolicy};

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
    }
}
