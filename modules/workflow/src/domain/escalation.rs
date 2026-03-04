//! Escalation timers — deterministic, exactly-once timer-driven escalations.
//!
//! Invariants:
//! - Only one active timer per (instance, rule) at a time.
//! - Firing a timer is atomic: Guard (unfired + due) → Mutation (set fired_at) → Outbox event.
//! - Idempotent: re-firing an already-fired timer is a no-op.
//! - Cancelled timers are never fired.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::events::{envelope, subjects};
use crate::outbox;

// ── Domain models ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EscalationRule {
    pub id: Uuid,
    pub tenant_id: String,
    pub definition_id: Uuid,
    pub step_id: String,
    pub timeout_seconds: i32,
    pub escalate_to_step: Option<String>,
    pub notify_actor_ids: Vec<Uuid>,
    pub notify_template: Option<String>,
    pub max_escalations: i32,
    pub is_active: bool,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EscalationTimer {
    pub id: Uuid,
    pub tenant_id: String,
    pub instance_id: Uuid,
    pub rule_id: Uuid,
    pub step_id: String,
    pub due_at: DateTime<Utc>,
    pub fired_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub escalation_count: i32,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ── Request types ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateEscalationRuleRequest {
    pub tenant_id: String,
    pub definition_id: Uuid,
    pub step_id: String,
    pub timeout_seconds: i32,
    pub escalate_to_step: Option<String>,
    pub notify_actor_ids: Vec<Uuid>,
    pub notify_template: Option<String>,
    pub max_escalations: Option<i32>,
    pub metadata: Option<serde_json::Value>,
}

// ── Errors ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum EscalationError {
    #[error("Rule not found")]
    RuleNotFound,

    #[error("Timer not found")]
    TimerNotFound,

    #[error("Timer already fired")]
    AlreadyFired,

    #[error("Timer already cancelled")]
    AlreadyCancelled,

    #[error("Max escalations reached ({0})")]
    MaxEscalationsReached(i32),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Event payloads ───────────────────────────────────────────

#[derive(Debug, Serialize)]
struct EscalationFiredPayload {
    timer_id: Uuid,
    instance_id: Uuid,
    tenant_id: String,
    rule_id: Uuid,
    step_id: String,
    escalation_count: i32,
    escalate_to_step: Option<String>,
    notify_actor_ids: Vec<Uuid>,
}

// ── Repository ───────────────────────────────────────────────

pub struct EscalationRepo;

impl EscalationRepo {
    /// Create an escalation rule for a step in a definition.
    /// Guard: definition must exist, timeout > 0, no duplicate rule for step.
    pub async fn create_rule(
        pool: &PgPool,
        req: &CreateEscalationRuleRequest,
    ) -> Result<EscalationRule, EscalationError> {
        if req.timeout_seconds <= 0 {
            return Err(EscalationError::Validation(
                "timeout_seconds must be positive".into(),
            ));
        }
        if req.step_id.is_empty() {
            return Err(EscalationError::Validation(
                "step_id is required".into(),
            ));
        }

        // Guard: definition exists
        let def_exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM workflow_definitions WHERE id = $1 AND tenant_id = $2",
        )
        .bind(req.definition_id)
        .bind(&req.tenant_id)
        .fetch_optional(pool)
        .await?;

        if def_exists.is_none() {
            return Err(EscalationError::Validation(
                "definition not found".into(),
            ));
        }

        let id = Uuid::new_v4();
        let max_esc = req.max_escalations.unwrap_or(1).max(1);

        let rule = sqlx::query_as::<_, EscalationRule>(
            r#"
            INSERT INTO workflow_escalation_rules
                (id, tenant_id, definition_id, step_id, timeout_seconds,
                 escalate_to_step, notify_actor_ids, notify_template,
                 max_escalations, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.definition_id)
        .bind(&req.step_id)
        .bind(req.timeout_seconds)
        .bind(&req.escalate_to_step)
        .bind(&req.notify_actor_ids)
        .bind(&req.notify_template)
        .bind(max_esc)
        .bind(&req.metadata)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err
                    .message()
                    .contains("duplicate key value violates unique constraint")
                {
                    return EscalationError::Validation(
                        "escalation rule already exists for this step".into(),
                    );
                }
            }
            EscalationError::Database(e)
        })?;

        Ok(rule)
    }

    /// Get an escalation rule by ID.
    pub async fn get_rule(
        pool: &PgPool,
        tenant_id: &str,
        rule_id: Uuid,
    ) -> Result<EscalationRule, EscalationError> {
        sqlx::query_as::<_, EscalationRule>(
            "SELECT * FROM workflow_escalation_rules WHERE id = $1 AND tenant_id = $2",
        )
        .bind(rule_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(EscalationError::RuleNotFound)
    }

    /// Find the active escalation rule for a given definition + step.
    pub async fn find_rule_for_step(
        pool: &PgPool,
        tenant_id: &str,
        definition_id: Uuid,
        step_id: &str,
    ) -> Result<Option<EscalationRule>, EscalationError> {
        Ok(sqlx::query_as::<_, EscalationRule>(
            r#"
            SELECT * FROM workflow_escalation_rules
            WHERE tenant_id = $1 AND definition_id = $2 AND step_id = $3 AND is_active = true
            "#,
        )
        .bind(tenant_id)
        .bind(definition_id)
        .bind(step_id)
        .fetch_optional(pool)
        .await?)
    }

    /// Arm a timer for an instance entering a step with an escalation rule.
    /// Idempotent: if an active timer already exists, return it.
    pub async fn arm_timer(
        pool: &PgPool,
        tenant_id: &str,
        instance_id: Uuid,
        rule: &EscalationRule,
    ) -> Result<EscalationTimer, EscalationError> {
        // Check for existing active timer
        let existing = sqlx::query_as::<_, EscalationTimer>(
            r#"
            SELECT * FROM workflow_escalation_timers
            WHERE instance_id = $1 AND rule_id = $2
              AND fired_at IS NULL AND cancelled_at IS NULL
            "#,
        )
        .bind(instance_id)
        .bind(rule.id)
        .fetch_optional(pool)
        .await?;

        if let Some(timer) = existing {
            return Ok(timer);
        }

        let id = Uuid::new_v4();
        let due_at = Utc::now() + chrono::Duration::seconds(rule.timeout_seconds as i64);
        let idem_key = format!("esc-arm-{}-{}-{}", instance_id, rule.id, Uuid::new_v4());

        let timer = sqlx::query_as::<_, EscalationTimer>(
            r#"
            INSERT INTO workflow_escalation_timers
                (id, tenant_id, instance_id, rule_id, step_id, due_at, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .bind(instance_id)
        .bind(rule.id)
        .bind(&rule.step_id)
        .bind(due_at)
        .bind(&idem_key)
        .fetch_one(pool)
        .await?;

        Ok(timer)
    }

    /// Cancel all active timers for an instance (e.g. when it advances past a step).
    pub async fn cancel_timers_for_instance(
        pool: &PgPool,
        tenant_id: &str,
        instance_id: Uuid,
    ) -> Result<Vec<EscalationTimer>, EscalationError> {
        let cancelled = sqlx::query_as::<_, EscalationTimer>(
            r#"
            UPDATE workflow_escalation_timers
            SET cancelled_at = now()
            WHERE tenant_id = $1 AND instance_id = $2
              AND fired_at IS NULL AND cancelled_at IS NULL
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(instance_id)
        .fetch_all(pool)
        .await?;

        Ok(cancelled)
    }

    /// Tick: find and fire all due timers. Exactly-once via Guard→Mutation→Outbox.
    /// Returns the list of escalation timers that were fired.
    pub async fn tick(
        pool: &PgPool,
        limit: i64,
    ) -> Result<Vec<EscalationTimer>, EscalationError> {
        Self::tick_inner(pool, limit, None).await
    }

    /// Tick scoped to a single tenant. Same semantics as `tick()` but only
    /// processes timers belonging to `tenant_id`. Prevents cross-tenant
    /// interference when multiple callers share the same database.
    pub async fn tick_for_tenant(
        pool: &PgPool,
        tenant_id: &str,
        limit: i64,
    ) -> Result<Vec<EscalationTimer>, EscalationError> {
        Self::tick_inner(pool, limit, Some(tenant_id)).await
    }

    async fn tick_inner(
        pool: &PgPool,
        limit: i64,
        tenant_id: Option<&str>,
    ) -> Result<Vec<EscalationTimer>, EscalationError> {
        let mut fired = Vec::new();
        let limit = limit.min(100);

        // Fetch due timers (not yet fired, not cancelled, due_at <= now)
        let due_timers = if let Some(tid) = tenant_id {
            sqlx::query_as::<_, EscalationTimer>(
                r#"
                SELECT * FROM workflow_escalation_timers
                WHERE tenant_id = $1 AND fired_at IS NULL AND cancelled_at IS NULL AND due_at <= now()
                ORDER BY due_at ASC
                LIMIT $2
                "#,
            )
            .bind(tid)
            .bind(limit)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_as::<_, EscalationTimer>(
                r#"
                SELECT * FROM workflow_escalation_timers
                WHERE fired_at IS NULL AND cancelled_at IS NULL AND due_at <= now()
                ORDER BY due_at ASC
                LIMIT $1
                "#,
            )
            .bind(limit)
            .fetch_all(pool)
            .await?
        };

        for timer in due_timers {
            match Self::fire_timer(pool, &timer).await {
                Ok(t) => fired.push(t),
                Err(EscalationError::AlreadyFired) => {
                    // Another process fired it — skip.
                }
                Err(EscalationError::MaxEscalationsReached(_)) => {
                    // Cancel the timer instead of firing.
                    let _ = sqlx::query(
                        r#"
                        UPDATE workflow_escalation_timers
                        SET cancelled_at = now()
                        WHERE id = $1 AND fired_at IS NULL AND cancelled_at IS NULL
                        "#,
                    )
                    .bind(timer.id)
                    .execute(pool)
                    .await;
                }
                Err(e) => {
                    tracing::error!(
                        timer_id = %timer.id,
                        error = %e,
                        "Failed to fire escalation timer"
                    );
                }
            }
        }

        Ok(fired)
    }

    /// Fire a single escalation timer. Atomic: Guard→Mutation→Outbox.
    /// Guard: timer must be active (unfired, uncancelled) and due.
    /// Mutation: set fired_at, increment escalation_count.
    /// Outbox: escalation.fired event.
    async fn fire_timer(
        pool: &PgPool,
        timer: &EscalationTimer,
    ) -> Result<EscalationTimer, EscalationError> {
        let mut tx = pool.begin().await?;

        // ── Guard: lock timer row, check still active ──
        let locked = sqlx::query_as::<_, EscalationTimer>(
            r#"
            SELECT * FROM workflow_escalation_timers
            WHERE id = $1 FOR UPDATE
            "#,
        )
        .bind(timer.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(EscalationError::TimerNotFound)?;

        if locked.fired_at.is_some() {
            return Err(EscalationError::AlreadyFired);
        }
        if locked.cancelled_at.is_some() {
            return Err(EscalationError::AlreadyCancelled);
        }

        // Guard: fetch rule to check max_escalations
        let rule = sqlx::query_as::<_, EscalationRule>(
            "SELECT * FROM workflow_escalation_rules WHERE id = $1",
        )
        .bind(locked.rule_id)
        .fetch_one(&mut *tx)
        .await?;

        // Count how many times this instance+step has been escalated
        let prev_count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM workflow_escalation_timers
            WHERE instance_id = $1 AND rule_id = $2 AND fired_at IS NOT NULL
            "#,
        )
        .bind(locked.instance_id)
        .bind(locked.rule_id)
        .fetch_one(&mut *tx)
        .await?;

        let new_count = (prev_count.0 as i32) + 1;
        if new_count > rule.max_escalations {
            return Err(EscalationError::MaxEscalationsReached(rule.max_escalations));
        }

        // ── Mutation: mark fired ──
        let updated = sqlx::query_as::<_, EscalationTimer>(
            r#"
            UPDATE workflow_escalation_timers
            SET fired_at = now(), escalation_count = $1
            WHERE id = $2
            RETURNING *
            "#,
        )
        .bind(new_count)
        .bind(locked.id)
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox: escalation.fired event ──
        let event_id = Uuid::new_v4();
        let payload = EscalationFiredPayload {
            timer_id: updated.id,
            instance_id: updated.instance_id,
            tenant_id: updated.tenant_id.clone(),
            rule_id: updated.rule_id,
            step_id: updated.step_id.clone(),
            escalation_count: new_count,
            escalate_to_step: rule.escalate_to_step.clone(),
            notify_actor_ids: rule.notify_actor_ids.clone(),
        };

        let env = envelope::create_envelope(
            event_id,
            updated.tenant_id.clone(),
            subjects::ESCALATION_FIRED.to_string(),
            payload,
        );
        let validated = envelope::validate_envelope(&env)
            .map_err(|e| EscalationError::Validation(format!("envelope: {}", e)))?;

        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::ESCALATION_FIRED,
            "workflow_escalation",
            &updated.id.to_string(),
            &validated,
        )
        .await?;

        tx.commit().await?;

        Ok(updated)
    }

    /// Get a timer by ID.
    pub async fn get_timer(
        pool: &PgPool,
        timer_id: Uuid,
    ) -> Result<EscalationTimer, EscalationError> {
        sqlx::query_as::<_, EscalationTimer>(
            "SELECT * FROM workflow_escalation_timers WHERE id = $1",
        )
        .bind(timer_id)
        .fetch_optional(pool)
        .await?
        .ok_or(EscalationError::TimerNotFound)
    }

    /// List active timers for an instance.
    pub async fn list_active_timers(
        pool: &PgPool,
        tenant_id: &str,
        instance_id: Uuid,
    ) -> Result<Vec<EscalationTimer>, EscalationError> {
        Ok(sqlx::query_as::<_, EscalationTimer>(
            r#"
            SELECT * FROM workflow_escalation_timers
            WHERE tenant_id = $1 AND instance_id = $2
              AND fired_at IS NULL AND cancelled_at IS NULL
            ORDER BY due_at ASC
            "#,
        )
        .bind(tenant_id)
        .bind(instance_id)
        .fetch_all(pool)
        .await?)
    }

    /// Arm a timer with a specific due_at timestamp (for testing).
    pub async fn arm_timer_with_due_at(
        pool: &PgPool,
        tenant_id: &str,
        instance_id: Uuid,
        rule: &EscalationRule,
        due_at: DateTime<Utc>,
    ) -> Result<EscalationTimer, EscalationError> {
        let existing = sqlx::query_as::<_, EscalationTimer>(
            r#"
            SELECT * FROM workflow_escalation_timers
            WHERE instance_id = $1 AND rule_id = $2
              AND fired_at IS NULL AND cancelled_at IS NULL
            "#,
        )
        .bind(instance_id)
        .bind(rule.id)
        .fetch_optional(pool)
        .await?;

        if let Some(timer) = existing {
            return Ok(timer);
        }

        let id = Uuid::new_v4();
        let idem_key = format!("esc-arm-{}-{}-{}", instance_id, rule.id, Uuid::new_v4());

        let timer = sqlx::query_as::<_, EscalationTimer>(
            r#"
            INSERT INTO workflow_escalation_timers
                (id, tenant_id, instance_id, rule_id, step_id, due_at, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .bind(instance_id)
        .bind(rule.id)
        .bind(&rule.step_id)
        .bind(due_at)
        .bind(&idem_key)
        .fetch_one(pool)
        .await?;

        Ok(timer)
    }
}
