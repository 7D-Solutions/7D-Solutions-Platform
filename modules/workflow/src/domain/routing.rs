//! Routing engine — sequential, parallel threshold, and conditional branching.
//!
//! Invariants:
//! - Deterministic: same inputs always produce same outputs (replay-safe).
//! - Deduped: duplicate actor decisions are rejected, not double-counted.
//! - Guard→Mutation→Outbox for every decision and auto-advance.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::instances::{InstanceError, WorkflowInstance};
use super::types::{BranchCondition, RoutingMode, StepDecision};

pub use super::routing_repo::RoutingEngine;

// ── Request types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RecordDecisionRequest {
    pub tenant_id: String,
    pub instance_id: Uuid,
    pub step_id: String,
    pub actor_id: Uuid,
    pub actor_type: Option<String>,
    pub decision: String,
    pub comment: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct EvaluateConditionRequest {
    pub tenant_id: String,
    pub instance_id: Uuid,
}

// ── Response types ────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DecisionResult {
    pub decision: StepDecision,
    pub threshold_met: bool,
    pub current_count: u32,
    pub threshold: u32,
    /// If threshold was met, contains the auto-advanced instance.
    pub advanced_instance: Option<WorkflowInstance>,
}

#[derive(Debug, Serialize)]
pub struct ConditionResult {
    pub matched_condition_index: Option<usize>,
    pub target_step: String,
    pub advanced_instance: Option<WorkflowInstance>,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("Instance not found")]
    InstanceNotFound,

    #[error("Instance is not active")]
    NotActive,

    #[error("Step mismatch: instance is at '{current}', decision is for '{requested}'")]
    StepMismatch { current: String, requested: String },

    #[error("Duplicate decision: actor {0} already decided on this step")]
    DuplicateDecision(Uuid),

    #[error("Routing mode not found for step '{0}' in definition")]
    RoutingModeNotFound(String),

    #[error("No condition matched and no default branch defined")]
    NoConditionMatched,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Instance error: {0}")]
    Instance(#[from] InstanceError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Helpers (pure logic, no SQL) ────────────────────────────

/// Extract the routing mode for a specific step from the definition's steps JSON.
pub(crate) fn extract_routing_mode(
    steps_json: &serde_json::Value,
    step_id: &str,
) -> Result<RoutingMode, RoutingError> {
    let steps = steps_json
        .as_array()
        .ok_or_else(|| RoutingError::Validation("steps must be a JSON array".into()))?;

    for step in steps {
        let sid = step.get("step_id").and_then(|v| v.as_str()).unwrap_or("");
        if sid == step_id {
            if let Some(rm) = step.get("routing_mode") {
                let routing: RoutingMode = serde_json::from_value(rm.clone())
                    .map_err(|e| RoutingError::Validation(format!("invalid routing_mode: {}", e)))?;
                return Ok(routing);
            }
            return Ok(RoutingMode::Sequential);
        }
    }

    Err(RoutingError::RoutingModeNotFound(step_id.into()))
}

/// Find the first allowed_transition for a parallel step (the target after threshold is met).
pub(crate) fn find_parallel_target(
    steps_json: &serde_json::Value,
    step_id: &str,
) -> Result<String, RoutingError> {
    let steps = steps_json
        .as_array()
        .ok_or_else(|| RoutingError::Validation("steps must be a JSON array".into()))?;

    for step in steps {
        let sid = step.get("step_id").and_then(|v| v.as_str()).unwrap_or("");
        if sid == step_id {
            if let Some(transitions) = step.get("allowed_transitions").and_then(|v| v.as_array()) {
                if let Some(first) = transitions.first().and_then(|v| v.as_str()) {
                    return Ok(first.to_string());
                }
            }
            return Err(RoutingError::Validation(format!(
                "parallel step '{}' has no allowed_transitions",
                step_id
            )));
        }
    }

    Err(RoutingError::RoutingModeNotFound(step_id.into()))
}

/// Evaluate branch conditions against instance context. Returns (matched_index, target_step).
/// Deterministic: evaluates conditions in order, first match wins. Default is fallback.
pub(crate) fn evaluate_conditions(
    conditions: &[BranchCondition],
    context: &serde_json::Value,
) -> Result<(Option<usize>, String), RoutingError> {
    let mut default_target: Option<String> = None;

    for (i, cond) in conditions.iter().enumerate() {
        if cond.default {
            default_target = Some(cond.target_step.clone());
            continue;
        }

        let field = cond
            .field
            .as_ref()
            .ok_or_else(|| RoutingError::Validation(format!("condition[{}] missing field", i)))?;
        let op = cond
            .op
            .as_ref()
            .ok_or_else(|| RoutingError::Validation(format!("condition[{}] missing op", i)))?;
        let expected = cond
            .value
            .as_ref()
            .ok_or_else(|| RoutingError::Validation(format!("condition[{}] missing value", i)))?;

        let actual = context.get(field.as_str());

        if matches_condition(actual, op, expected) {
            return Ok((Some(i), cond.target_step.clone()));
        }
    }

    match default_target {
        Some(target) => Ok((None, target)),
        None => Err(RoutingError::NoConditionMatched),
    }
}

/// Compare a context value against an expected value using the given operator.
fn matches_condition(
    actual: Option<&serde_json::Value>,
    op: &str,
    expected: &serde_json::Value,
) -> bool {
    let actual = match actual {
        Some(v) => v,
        None => return false,
    };

    match op {
        "eq" => actual == expected,
        "neq" => actual != expected,
        "gt" => compare_numbers(actual, expected).is_some_and(|o| o == std::cmp::Ordering::Greater),
        "gte" => {
            compare_numbers(actual, expected).is_some_and(|o| o != std::cmp::Ordering::Less)
        }
        "lt" => compare_numbers(actual, expected).is_some_and(|o| o == std::cmp::Ordering::Less),
        "lte" => {
            compare_numbers(actual, expected).is_some_and(|o| o != std::cmp::Ordering::Greater)
        }
        "in" => {
            if let Some(arr) = expected.as_array() {
                arr.contains(actual)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn compare_numbers(
    a: &serde_json::Value,
    b: &serde_json::Value,
) -> Option<std::cmp::Ordering> {
    let a_num = a.as_f64()?;
    let b_num = b.as_f64()?;
    a_num.partial_cmp(&b_num)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_sequential_default() {
        let steps = json!([{ "step_id": "draft", "name": "Draft" }]);
        let mode = extract_routing_mode(&steps, "draft").expect("extract failed");
        assert_eq!(mode, RoutingMode::Sequential);
    }

    #[test]
    fn extract_parallel_mode() {
        let steps = json!([{
            "step_id": "review",
            "routing_mode": { "mode": "parallel", "threshold": 2 }
        }]);
        let mode = extract_routing_mode(&steps, "review").expect("extract failed");
        assert_eq!(mode, RoutingMode::Parallel { threshold: 2 });
    }

    #[test]
    fn extract_conditional_mode() {
        let steps = json!([{
            "step_id": "triage",
            "routing_mode": {
                "mode": "conditional",
                "conditions": [
                    { "field": "amount", "op": "gte", "value": 10000, "target_step": "director" },
                    { "default": true, "target_step": "manager" }
                ]
            }
        }]);
        let mode = extract_routing_mode(&steps, "triage").expect("extract failed");
        match mode {
            RoutingMode::Conditional { conditions } => assert_eq!(conditions.len(), 2),
            _ => panic!("expected conditional"),
        }
    }

    #[test]
    fn condition_evaluation_gte() {
        let conditions = vec![
            BranchCondition {
                field: Some("amount".into()),
                op: Some("gte".into()),
                value: Some(json!(10000)),
                target_step: "director".into(),
                default: false,
            },
            BranchCondition {
                field: None,
                op: None,
                value: None,
                target_step: "manager".into(),
                default: true,
            },
        ];

        // Amount >= 10000 → director
        let ctx = json!({ "amount": 15000 });
        let (idx, target) = evaluate_conditions(&conditions, &ctx).expect("eval failed");
        assert_eq!(idx, Some(0));
        assert_eq!(target, "director");

        // Amount < 10000 → manager (default)
        let ctx2 = json!({ "amount": 500 });
        let (idx2, target2) = evaluate_conditions(&conditions, &ctx2).expect("eval failed");
        assert_eq!(idx2, None);
        assert_eq!(target2, "manager");
    }

    #[test]
    fn condition_evaluation_eq() {
        let conditions = vec![
            BranchCondition {
                field: Some("priority".into()),
                op: Some("eq".into()),
                value: Some(json!("high")),
                target_step: "urgent_review".into(),
                default: false,
            },
            BranchCondition {
                field: None,
                op: None,
                value: None,
                target_step: "normal_review".into(),
                default: true,
            },
        ];

        let ctx = json!({ "priority": "high" });
        let (_, target) = evaluate_conditions(&conditions, &ctx).expect("eval failed");
        assert_eq!(target, "urgent_review");

        let ctx2 = json!({ "priority": "low" });
        let (_, target2) = evaluate_conditions(&conditions, &ctx2).expect("eval failed");
        assert_eq!(target2, "normal_review");
    }

    #[test]
    fn condition_no_match_no_default_fails() {
        let conditions = vec![BranchCondition {
            field: Some("x".into()),
            op: Some("eq".into()),
            value: Some(json!(1)),
            target_step: "a".into(),
            default: false,
        }];

        let ctx = json!({ "x": 2 });
        let result = evaluate_conditions(&conditions, &ctx);
        assert!(matches!(result, Err(RoutingError::NoConditionMatched)));
    }

    #[test]
    fn condition_in_operator() {
        let conditions = vec![BranchCondition {
            field: Some("region".into()),
            op: Some("in".into()),
            value: Some(json!(["US", "CA"])),
            target_step: "north_america".into(),
            default: false,
        }];

        let ctx = json!({ "region": "US" });
        let (idx, target) = evaluate_conditions(&conditions, &ctx).expect("eval failed");
        assert_eq!(idx, Some(0));
        assert_eq!(target, "north_america");
    }
}
