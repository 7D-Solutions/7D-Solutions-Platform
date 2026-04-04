//! Routing repository — all SQL for workflow routing decisions and transitions.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use super::definitions::WorkflowDefinition;
use super::instances::{AdvanceInstanceRequest, WorkflowInstance};
use super::instances_repo::InstanceRepo;
use super::routing::{
    evaluate_conditions, extract_routing_mode, find_parallel_target, ConditionResult,
    DecisionResult, EvaluateConditionRequest, RecordDecisionRequest, RoutingError,
};
use super::types::{InstanceStatus, RoutingMode, StepDecision};
use crate::events::{envelope, subjects};
use crate::outbox;

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

        if instance.status != InstanceStatus::Active {
            return Err(RoutingError::NotActive);
        }

        if instance.current_step_id != req.step_id {
            return Err(RoutingError::StepMismatch {
                current: instance.current_step_id.clone(),
                requested: req.step_id.clone(),
            });
        }

        // ── Guard: read routing mode from definition ──
        let def = sqlx::query_as::<_, WorkflowDefinition>(
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

        if instance.status != InstanceStatus::Active {
            return Err(RoutingError::NotActive);
        }

        let def = sqlx::query_as::<_, WorkflowDefinition>(
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
