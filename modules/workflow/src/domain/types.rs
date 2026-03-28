//! Core workflow types and status enums.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    Active,
    Completed,
    Cancelled,
}

impl fmt::Display for InstanceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstanceStatus::Active => write!(f, "active"),
            InstanceStatus::Completed => write!(f, "completed"),
            InstanceStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl TryFrom<String> for InstanceStatus {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "active" => Ok(InstanceStatus::Active),
            "completed" => Ok(InstanceStatus::Completed),
            "cancelled" => Ok(InstanceStatus::Cancelled),
            _ => Err(format!("Invalid instance status: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDefinition {
    pub step_id: String,
    pub name: String,
    pub step_type: String,
    pub position: i32,
}

// ── Routing mode ─────────────────────────────────────────────

/// How a step collects decisions and determines the next step.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RoutingMode {
    /// Single decision advances to the specified target step (default).
    #[default]
    Sequential,
    /// N-of-M actors must decide before advancing.
    Parallel {
        threshold: u32,
    },
    /// Evaluate conditions against instance context to pick a branch.
    Conditional {
        conditions: Vec<BranchCondition>,
    },
}

/// A single condition→target mapping for conditional routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchCondition {
    /// JSON-path–style field name in instance context (e.g. "amount").
    pub field: Option<String>,
    /// Comparison operator: "eq", "neq", "gt", "gte", "lt", "lte", "in".
    pub op: Option<String>,
    /// Value to compare against (JSON scalar or array for "in").
    pub value: Option<serde_json::Value>,
    /// The step to route to if this condition matches.
    pub target_step: String,
    /// If true, this is the fallback when no other condition matches.
    #[serde(default)]
    pub default: bool,
}

// ── Step decision ────────────────────────────────────────────

/// A single actor's decision at a parallel step.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct StepDecision {
    pub id: Uuid,
    pub tenant_id: String,
    pub instance_id: Uuid,
    pub step_id: String,
    pub actor_id: Uuid,
    pub actor_type: String,
    pub decision: String,
    pub comment: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub decided_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_status_roundtrip() {
        let status = InstanceStatus::Active;
        assert_eq!(status.to_string(), "active");
        assert_eq!(
            InstanceStatus::try_from("active".to_string()).unwrap(),
            InstanceStatus::Active
        );
    }

    #[test]
    fn instance_status_invalid() {
        assert!(InstanceStatus::try_from("bogus".to_string()).is_err());
    }
}
