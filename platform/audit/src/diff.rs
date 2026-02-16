/// Audit diff (before/after state changes)
///
/// Placeholder for change tracking and diff generation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diff {
    pub before: Option<Value>,
    pub after: Option<Value>,
}

impl Diff {
    pub fn new(before: Option<Value>, after: Option<Value>) -> Self {
        Self { before, after }
    }

    pub fn is_creation(&self) -> bool {
        self.before.is_none() && self.after.is_some()
    }

    pub fn is_deletion(&self) -> bool {
        self.before.is_some() && self.after.is_none()
    }

    pub fn is_modification(&self) -> bool {
        self.before.is_some() && self.after.is_some()
    }
}
