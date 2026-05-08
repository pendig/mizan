use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtkFilterResult {
    pub original_bytes: usize,
    pub filtered_bytes: usize,
    pub body: String,
}

pub fn passthrough_filter(output: impl Into<String>) -> RtkFilterResult {
    let body = output.into();
    let size = body.len();

    RtkFilterResult {
        original_bytes: size,
        filtered_bytes: size,
        body,
    }
}
