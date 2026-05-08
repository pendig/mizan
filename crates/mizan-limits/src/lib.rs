use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LimitScope {
    ApiKey(Uuid),
    User(Uuid),
    Provider(Uuid),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LimitDecision {
    Allowed,
    Blocked { reason: String },
}

impl LimitDecision {
    pub fn allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}
