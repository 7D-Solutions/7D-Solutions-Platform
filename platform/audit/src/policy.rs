/// Audit policy (retention, access control)
///
/// Placeholder for audit retention policies and access control.

use serde::{Deserialize, Serialize};
use chrono::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub retention_days: i64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            retention_days: 2555, // 7 years default
        }
    }
}

impl RetentionPolicy {
    pub fn duration(&self) -> Duration {
        Duration::days(self.retention_days)
    }
}
