//! Routing engine — sequential, parallel threshold, and conditional branching.
//!
//! Invariants:
//! - Deterministic: same inputs always produce same outputs (replay-safe).
//! - Deduped: duplicate actor decisions are rejected, not double-counted.
//! - Guard→Mutation→Outbox for every decision and auto-advance.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use super::instances::{AdvanceInstanceRequest, InstanceError, InstanceRepo, WorkflowInstance};
use super::types::{BranchCondition, RoutingMode, StepDecision};
use crate::events::{envelope, subjects};
use crate::outbox;

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

// ── Event payloads ────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct DecisionRecordedPayload {
    instance_id: Uuid,
    tenant_id: String,
    step_id: String,
    actor_id: Uuid,
    decision: String,
    current_count: u32,
    threshold: u32,
}

#[derive(Debug, Serialize)]
struct ThresholdMetPayload {
    instance_id: Uuid,
    tenant_id: String,
    step_id: String,
    decision_count: u32,
    threshold: u32,
    target_step: String,
}

// ── Routing engine ────────────────────────────────────────────

pub struct RoutingEngine;

impl RoutingEngine {
    /// Record a decision at a parallel step.
    /// Guard: instance active, step matches, actor hasn't already decided.
    /// Mutation: INSERT decision row.
    /// Outbox: decision_recorded event. If threshold met, auto-advance + threshold_met event.
    pub async fn record_decision(
        pool: &PgPool,
        req: &RecordDecisionRequest,
    ) -> Result<DecisionResult, RoutingError> {
        let mut tx = pool.begin().await?;

        // ── Guard: fetch and lock instance ──
        let instance = sqlx::query_as::<_, WorkflowInstance>(
            "SELECT * FROM workflow_instances WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(req.instance_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RoutingError::InstanceNotFound)?;

        if instance.status != super::types::InstanceStatus::Active {
            return Err(RoutingError::NotActive);
        }

        if instance.current_step_id != req.step_id {
            return Err(RoutingError::StepMismatch {
                current: instance.current_step_id.clone(),
                requested: req.step_id.clone(),
            });
        }

        // ── Guard: read routing mode from definition ──
        let def = sqlx::query_as::<_, super::definitions::WorkflowDefinition>(
            "SELECT * FROM workflow_definitions WHERE id = $1 AND tenant_id = $2",
        )
        .bind(instance.definition_id)
        .bind(&req.tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        let routing = extract_routing_mode(&def.steps, &req.step_id)?;
        let threshold = match &routing {
            RoutingMode::Parallel { threshold } => *threshold,
            _ => {
                return Err(RoutingError::Validation(
                    "record_decision is only valid for parallel routing steps".into(),
                ));
            }
        };

        // ── Guard: check for duplicate (actor already decided on this step) ──
        let existing = sqlx::query_as::<_, StepDecision>(
            r#"
            SELECT * FROM workflow_step_decisions
            WHERE instance_id = $1 AND step_id = $2 AND actor_id = $3
            "#,
        )
        .bind(req.instance_id)
        .bind(&req.step_id)
        .bind(req.actor_id)
        .fetch_optional(&mut *tx)
        .await?;

        if existing.is_some() {
            return Err(RoutingError::DuplicateDecision(req.actor_id));
        }

        // ── Mutation: insert decision ──
        let decision_id = Uuid::new_v4();
        let actor_type = req.actor_type.clone().unwrap_or_else(|| "user".into());

        let decision = sqlx::query_as::<_, StepDecision>(
            r#"
            INSERT INTO workflow_step_decisions
                (id, tenant_id, instance_id, step_id, actor_id, actor_type,
                 decision, comment, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(decision_id)
        .bind(&req.tenant_id)
        .bind(req.instance_id)
        .bind(&req.step_id)
        .bind(req.actor_id)
        .bind(&actor_type)
        .bind(&req.decision)
        .bind(&req.comment)
        .bind(&req.metadata)
        .fetch_one(&mut *tx)
        .await?;

        // ── Count decisions for this step ──
        let count_row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM workflow_step_decisions WHERE instance_id = $1 AND step_id = $2",
        )
        .bind(req.instance_id)
        .bind(&req.step_id)
        .fetch_one(&mut *tx)
        .await?;
        let current_count = count_row.0 as u32;

        // ── Outbox: decision recorded event ──
        let event_id = Uuid::new_v4();
        let payload = DecisionRecordedPayload {
            instance_id: req.instance_id,
            tenant_id: req.tenant_id.clone(),
            step_id: req.step_id.clone(),
            actor_id: req.actor_id,
            decision: req.decision.clone(),
            current_count,
            threshold,
        };
        let env = envelope::create_envelope(
            event_id,
            req.tenant_id.clone(),
            subjects::STEP_DECISION_RECORDED.to_string(),
            payload,
        );
        let validated = envelope::validate_envelope(&env)
            .map_err(|e| RoutingError::Validation(format!("envelope: {}", e)))?;
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::STEP_DECISION_RECORDED,
            "workflow_instance",
            &req.instance_id.to_string(),
            &validated,
        )
        .await?;

        // ── Check threshold ──
        let threshold_met = current_count >= threshold;
        let mut advanced_instance = None;

        if threshold_met {
            // Find target step: look for allowed_transitions on this step
            let target = find_parallel_target(&def.steps, &req.step_id)?;

            // Auto-advance the instance
            let updated = sqlx::query_as::<_, WorkflowInstance>(
                r#"
                UPDATE workflow_instances
                SET current_step_id = $1, updated_at = now()
                WHERE id = $2 AND tenant_id = $3
                RETURNING *
                "#,
            )
            .bind(&target)
            .bind(req.instance_id)
            .bind(&req.tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            // Record the transition
            let transition_id = Uuid::new_v4();
            sqlx::query(
                r#"
                INSERT INTO workflow_transitions
                    (id, tenant_id, instance_id, from_step_id, to_step_id, action)
                VALUES ($1, $2, $3, $4, $5, 'parallel_threshold_met')
                "#,
            )
            .bind(transition_id)
            .bind(&req.tenant_id)
            .bind(req.instance_id)
            .bind(&req.step_id)
            .bind(&target)
            .execute(&mut *tx)
            .await?;

            // Outbox: threshold met event
            let thresh_event_id = Uuid::new_v4();
            let thresh_payload = ThresholdMetPayload {
                instance_id: req.instance_id,
                tenant_id: req.tenant_id.clone(),
                step_id: req.step_id.clone(),
                decision_count: current_count,
                threshold,
                target_step: target,
            };
            let env2 = envelope::create_envelope(
                thresh_event_id,
                req.tenant_id.clone(),
                subjects::PARALLEL_THRESHOLD_MET.to_string(),
                thresh_payload,
            );
            let validated2 = envelope::validate_envelope(&env2)
                .map_err(|e| RoutingError::Validation(format!("envelope: {}", e)))?;
            outbox::enqueue_event_tx(
                &mut tx,
                thresh_event_id,
                subjects::PARALLEL_THRESHOLD_MET,
                "workflow_instance",
                &req.instance_id.to_string(),
                &validated2,
            )
            .await?;

            advanced_instance = Some(updated);
        }

        tx.commit().await?;

        Ok(DecisionResult {
            decision,
            threshold_met,
            current_count,
            threshold,
            advanced_instance,
        })
    }

    /// Evaluate conditional routing and auto-advance the instance.
    /// Guard: instance active, step has conditional routing mode.
    /// Mutation: advance instance to the matched branch's target step.
    /// Outbox: instance.advanced event.
    pub async fn evaluate_condition(
        pool: &PgPool,
        req: &EvaluateConditionRequest,
    ) -> Result<ConditionResult, RoutingError> {
        let instance =
            InstanceRepo::get(pool, &req.tenant_id, req.instance_id).await?;

        if instance.status != super::types::InstanceStatus::Active {
            return Err(RoutingError::NotActive);
        }

        let def = sqlx::query_as::<_, super::definitions::WorkflowDefinition>(
            "SELECT * FROM workflow_definitions WHERE id = $1 AND tenant_id = $2",
        )
        .bind(instance.definition_id)
        .bind(&req.tenant_id)
        .fetch_one(pool)
        .await?;

        let routing = extract_routing_mode(&def.steps, &instance.current_step_id)?;
        let conditions = match &routing {
            RoutingMode::Conditional { conditions } => conditions,
            _ => {
                return Err(RoutingError::Validation(
                    "evaluate_condition is only valid for conditional routing steps".into(),
                ));
            }
        };

        let (idx, target) = evaluate_conditions(conditions, &instance.context)?;

        // Auto-advance via the standard advance mechanism
        let (advanced, _) = InstanceRepo::advance(
            pool,
            req.instance_id,
            &AdvanceInstanceRequest {
                tenant_id: req.tenant_id.clone(),
                to_step_id: target.clone(),
                action: "conditional_branch".into(),
                actor_id: None,
                actor_type: Some("system".into()),
                comment: Some(format!("Condition matched at index {:?}", idx)),
                metadata: None,
                idempotency_key: None,
            },
        )
        .await?;

        Ok(ConditionResult {
            matched_condition_index: idx,
            target_step: target,
            advanced_instance: Some(advanced),
        })
    }

    /// List decisions recorded for a given instance + step.
    pub async fn list_decisions(
        pool: &PgPool,
        tenant_id: &str,
        instance_id: Uuid,
        step_id: &str,
    ) -> Result<Vec<StepDecision>, RoutingError> {
        Ok(sqlx::query_as::<_, StepDecision>(
            r#"
            SELECT * FROM workflow_step_decisions
            WHERE tenant_id = $1 AND instance_id = $2 AND step_id = $3
            ORDER BY decided_at ASC
            "#,
        )
        .bind(tenant_id)
        .bind(instance_id)
        .bind(step_id)
        .fetch_all(pool)
        .await?)
    }
}

// ── Helpers ──────────────────────────────────────────────────

/// Extract the routing mode for a specific step from the definition's steps JSON.
fn extract_routing_mode(
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
fn find_parallel_target(
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
fn evaluate_conditions(
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
