//! AR Dunning Scheduler Worker (bd-2bj)
//!
//! Polls for due dunning rows using `FOR UPDATE SKIP LOCKED` to claim work,
//! executes the next dunning action, records the outcome, computes backoff,
//! and emits state-changed events atomically — all within a single transaction.
//!
//! ## Concurrency Safety
//!
//! - `FOR UPDATE SKIP LOCKED` ensures two concurrent workers never process the same row.
//! - Claim + state update + outbox event all happen in ONE transaction.
//! - Bounded exponential backoff: base 1h, factor 2×, max 72h.
//!
//! ## Backoff Formula
//!
//! ```text
//! next_attempt_at = now + min(base * 2^(attempt_count - 1), max_delay)
//! ```
//!
//! Where base = 1 hour, max_delay = 72 hours.

use crate::dunning::{DunningError, DunningStateValue};
use crate::events::{
    build_dunning_state_changed_envelope, build_invoice_suspended_envelope, DunningStateChangedPayload, InvoiceSuspendedPayload,
    EVENT_TYPE_DUNNING_STATE_CHANGED, EVENT_TYPE_INVOICE_SUSPENDED,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Backoff configuration
// ============================================================================

/// Base delay for the first retry (1 hour)
const BACKOFF_BASE_SECS: i64 = 3600;

/// Maximum delay cap (72 hours)
const BACKOFF_MAX_SECS: i64 = 72 * 3600;

/// Compute the next attempt time using bounded exponential backoff.
///
/// Formula: `now + min(base * 2^(attempt - 1), max_delay)`
///
/// - attempt=1 → 1h
/// - attempt=2 → 2h
/// - attempt=3 → 4h
/// - attempt=4 → 8h
/// - ...
/// - attempt=7+ → capped at 72h
pub fn compute_next_attempt(now: DateTime<Utc>, attempt_count: i32) -> DateTime<Utc> {
    let exponent = (attempt_count - 1).max(0) as u32;
    let delay_secs = BACKOFF_BASE_SECS.saturating_mul(2_i64.saturating_pow(exponent));
    let capped = delay_secs.min(BACKOFF_MAX_SECS);
    now + Duration::seconds(capped)
}

// ============================================================================
// Claimable row
// ============================================================================

/// A dunning row that is due for processing.
#[derive(Debug)]
struct ClaimableDunningRow {
    id: i32,
    dunning_id: Uuid,
    app_id: String,
    invoice_id: i32,
    customer_id: String,
    state: String,
    version: i32,
    attempt_count: i32,
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for ClaimableDunningRow {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            dunning_id: row.try_get("dunning_id")?,
            app_id: row.try_get("app_id")?,
            invoice_id: row.try_get("invoice_id")?,
            customer_id: row.try_get("customer_id")?,
            state: row.try_get("state")?,
            version: row.try_get("version")?,
            attempt_count: row.try_get("attempt_count")?,
        })
    }
}

// ============================================================================
// Execution outcome
// ============================================================================

/// Outcome of a single dunning execution step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DunningExecutionOutcome {
    /// Transitioned to the next state successfully
    Transitioned {
        from_state: String,
        to_state: String,
        new_attempt_count: i32,
        next_attempt_at: Option<DateTime<Utc>>,
    },
    /// Row was already in a terminal state — no action taken
    AlreadyTerminal { state: String },
    /// No rows were due — nothing to process
    NothingToClaim,
    /// An error occurred during execution
    Failed { error: String },
}

// ============================================================================
// State progression logic
// ============================================================================

/// Determine the next dunning state for a given current state.
///
/// This is the scheduler's progression policy:
/// - Pending → Warned (first collection notice)
/// - Warned → Escalated (second notice, stronger language)
/// - Escalated → Suspended (service interruption)
/// - Suspended / Resolved / WrittenOff → None (terminal, no further progression)
fn next_state_for(current: &DunningStateValue) -> Option<DunningStateValue> {
    match current {
        DunningStateValue::Pending => Some(DunningStateValue::Warned),
        DunningStateValue::Warned => Some(DunningStateValue::Escalated),
        DunningStateValue::Escalated => Some(DunningStateValue::Suspended),
        // Terminal or suspended — no automatic progression
        DunningStateValue::Suspended
        | DunningStateValue::Resolved
        | DunningStateValue::WrittenOff => None,
    }
}

// ============================================================================
// Core scheduler functions
// ============================================================================

/// Claim and execute a single due dunning row within a single transaction.
///
/// Uses `FOR UPDATE SKIP LOCKED` to safely claim one row that is due
/// (next_attempt_at <= now, non-terminal state). Within the same transaction:
/// 1. Determines the next state via progression policy
/// 2. Computes bounded backoff for next_attempt_at
/// 3. Updates the state row
/// 4. Inserts outbox event (LIFECYCLE)
/// 5. Commits atomically
///
/// An optional `app_id` filter restricts claiming to a specific tenant.
pub async fn claim_and_execute_one(
    pool: &PgPool,
    correlation_id: &str,
    app_id_filter: Option<&str>,
) -> Result<DunningExecutionOutcome, DunningError> {
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    // 1. Claim one due row with SKIP LOCKED (inside this transaction)
    let row: Option<ClaimableDunningRow> = if let Some(app_id) = app_id_filter {
        sqlx::query_as(
            r#"
            SELECT id, dunning_id, app_id, invoice_id, customer_id, state, version, attempt_count
            FROM ar_dunning_states
            WHERE next_attempt_at <= $1
              AND state NOT IN ('resolved', 'written_off')
              AND app_id = $2
            ORDER BY next_attempt_at ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
            "#,
        )
        .bind(now)
        .bind(app_id)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        sqlx::query_as(
            r#"
            SELECT id, dunning_id, app_id, invoice_id, customer_id, state, version, attempt_count
            FROM ar_dunning_states
            WHERE next_attempt_at <= $1
              AND state NOT IN ('resolved', 'written_off')
            ORDER BY next_attempt_at ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
            "#,
        )
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?
    };

    let row = match row {
        Some(r) => r,
        None => {
            tx.rollback().await?;
            return Ok(DunningExecutionOutcome::NothingToClaim);
        }
    };

    let current_state = match DunningStateValue::from_str(&row.state) {
        Some(s) => s,
        None => {
            tx.rollback().await?;
            return Ok(DunningExecutionOutcome::Failed {
                error: format!("Unknown state in DB: {}", row.state),
            });
        }
    };

    // 2. Determine next state
    let target_state = match next_state_for(&current_state) {
        Some(s) => s,
        None => {
            tx.rollback().await?;
            return Ok(DunningExecutionOutcome::AlreadyTerminal {
                state: row.state.clone(),
            });
        }
    };

    // 3. Compute backoff for next attempt
    let new_attempt_count = row.attempt_count + 1;
    let next_attempt_at = if target_state.is_terminal() {
        None
    } else {
        Some(compute_next_attempt(now, new_attempt_count))
    };

    // 4. Update the state row (optimistic lock via version)
    let rows_updated = sqlx::query(
        r#"
        UPDATE ar_dunning_states
        SET
            state           = $1,
            version         = version + 1,
            attempt_count   = $2,
            next_attempt_at = $3,
            updated_at      = $4
        WHERE
            id = $5
            AND version = $6
        "#,
    )
    .bind(target_state.as_str())
    .bind(new_attempt_count)
    .bind(next_attempt_at)
    .bind(now)
    .bind(row.id)
    .bind(row.version)
    .execute(&mut *tx)
    .await?;

    if rows_updated.rows_affected() == 0 {
        tx.rollback().await?;
        return Err(DunningError::ConcurrentModification {
            invoice_id: row.invoice_id,
        });
    }

    // 5. Build and enqueue outbox event (LIFECYCLE)
    let outbox_event_id = Uuid::new_v4();
    let payload = DunningStateChangedPayload {
        tenant_id: row.app_id.clone(),
        invoice_id: row.invoice_id.to_string(),
        customer_id: row.customer_id.clone(),
        from_state: Some(current_state.to_event_state()),
        to_state: target_state.to_event_state(),
        reason: format!("scheduler_auto_escalation_attempt_{}", new_attempt_count),
        attempt_number: new_attempt_count,
        next_retry_at: next_attempt_at,
        transitioned_at: now,
    };

    let envelope = build_dunning_state_changed_envelope(
        outbox_event_id,
        row.app_id.clone(),
        correlation_id.to_string(),
        Some(format!("dunning-scheduler-{}", row.dunning_id)),
        payload,
    );

    let payload_json = serde_json::to_value(&envelope)
        .map_err(|e| DunningError::DatabaseError(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, mutation_class, schema_version,
            occurred_at, replay_safe, correlation_id, causation_id
        )
        VALUES ($1, $2, 'dunning_state', $3, $4, $5, 'ar', 'LIFECYCLE', $6, $7, true, $8, $9)
        "#,
    )
    .bind(outbox_event_id)
    .bind(EVENT_TYPE_DUNNING_STATE_CHANGED)
    .bind(row.dunning_id.to_string())
    .bind(payload_json)
    .bind(&row.app_id)
    .bind(&envelope.schema_version)
    .bind(now)
    .bind(correlation_id)
    .bind(format!("dunning-scheduler-{}", row.dunning_id))
    .execute(&mut *tx)
    .await?;

    // 5b. If transitioning to Suspended, also emit ar.invoice_suspended
    if target_state == DunningStateValue::Suspended {
        let suspended_event_id = Uuid::new_v4();
        let suspended_payload = InvoiceSuspendedPayload {
            tenant_id: row.app_id.clone(),
            invoice_id: row.invoice_id.to_string(),
            customer_id: row.customer_id.clone(),
            outstanding_minor: 0,
            currency: String::new(),
            dunning_attempt: new_attempt_count,
            reason: format!("scheduler_auto_escalation_attempt_{}", new_attempt_count),
            grace_period_ends_at: None,
            suspended_at: now,
        };

        let suspended_envelope = build_invoice_suspended_envelope(
            suspended_event_id,
            row.app_id.clone(),
            correlation_id.to_string(),
            Some(outbox_event_id.to_string()),
            suspended_payload,
        );

        let suspended_json = serde_json::to_value(&suspended_envelope)
            .map_err(|e| DunningError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO events_outbox (
                event_id, event_type, aggregate_type, aggregate_id, payload,
                tenant_id, source_module, mutation_class, schema_version,
                occurred_at, replay_safe, correlation_id, causation_id
            )
            VALUES ($1, $2, 'invoice', $3, $4, $5, 'ar', 'LIFECYCLE', $6, $7, true, $8, $9)
            "#,
        )
        .bind(suspended_event_id)
        .bind(EVENT_TYPE_INVOICE_SUSPENDED)
        .bind(row.invoice_id.to_string())
        .bind(suspended_json)
        .bind(&row.app_id)
        .bind(&suspended_envelope.schema_version)
        .bind(now)
        .bind(correlation_id)
        .bind(outbox_event_id.to_string())
        .execute(&mut *tx)
        .await?;
    }

    // 6. Update outbox_event_id on the dunning record for correlation
    sqlx::query(
        "UPDATE ar_dunning_states SET outbox_event_id = $1 WHERE id = $2",
    )
    .bind(outbox_event_id)
    .bind(row.id)
    .execute(&mut *tx)
    .await?;

    // 7. Commit the entire transaction atomically
    tx.commit().await?;

    Ok(DunningExecutionOutcome::Transitioned {
        from_state: current_state.as_str().to_string(),
        to_state: target_state.as_str().to_string(),
        new_attempt_count,
        next_attempt_at,
    })
}

/// Poll and execute a batch of due dunning rows.
///
/// Calls `claim_and_execute_one` up to `batch_size` times, stopping early
/// when there's nothing left to claim. Returns all outcomes.
///
/// An optional `app_id` filter restricts claiming to a specific tenant.
pub async fn poll_and_execute_batch(
    pool: &PgPool,
    batch_size: usize,
    correlation_id: &str,
    app_id_filter: Option<&str>,
) -> Vec<DunningExecutionOutcome> {
    let mut outcomes = Vec::with_capacity(batch_size);

    for _ in 0..batch_size {
        match claim_and_execute_one(pool, correlation_id, app_id_filter).await {
            Ok(DunningExecutionOutcome::NothingToClaim) => {
                outcomes.push(DunningExecutionOutcome::NothingToClaim);
                break; // No more work available
            }
            Ok(outcome) => outcomes.push(outcome),
            Err(e) => {
                outcomes.push(DunningExecutionOutcome::Failed {
                    error: e.to_string(),
                });
            }
        }
    }

    outcomes
}

// ============================================================================
// Unit tests (pure logic — no DB)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn backoff_attempt_1_is_1h() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 1);
        let diff = (next - now).num_seconds();
        assert_eq!(diff, 3600, "attempt 1 should be 1 hour");
    }

    #[test]
    fn backoff_attempt_2_is_2h() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 2);
        let diff = (next - now).num_seconds();
        assert_eq!(diff, 7200, "attempt 2 should be 2 hours");
    }

    #[test]
    fn backoff_attempt_3_is_4h() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 3);
        let diff = (next - now).num_seconds();
        assert_eq!(diff, 14400, "attempt 3 should be 4 hours");
    }

    #[test]
    fn backoff_attempt_7_is_64h() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 7);
        let diff = (next - now).num_seconds();
        // 2^6 * 3600 = 230400 = 64h, still under 72h cap
        assert_eq!(diff, 230400, "attempt 7 should be 64 hours");
    }

    #[test]
    fn backoff_attempt_8_caps_at_72h() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 8);
        let diff = (next - now).num_seconds();
        // 2^7 * 3600 = 460800 > 259200 (72h), should cap
        assert_eq!(diff, BACKOFF_MAX_SECS, "attempt 8 should cap at 72 hours");
    }

    #[test]
    fn backoff_attempt_0_treated_as_attempt_1() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 0);
        let diff = (next - now).num_seconds();
        assert_eq!(diff, 3600, "attempt 0 should be treated as base (1h)");
    }

    #[test]
    fn state_progression_policy() {
        assert_eq!(
            next_state_for(&DunningStateValue::Pending),
            Some(DunningStateValue::Warned)
        );
        assert_eq!(
            next_state_for(&DunningStateValue::Warned),
            Some(DunningStateValue::Escalated)
        );
        assert_eq!(
            next_state_for(&DunningStateValue::Escalated),
            Some(DunningStateValue::Suspended)
        );
        assert_eq!(next_state_for(&DunningStateValue::Suspended), None);
        assert_eq!(next_state_for(&DunningStateValue::Resolved), None);
        assert_eq!(next_state_for(&DunningStateValue::WrittenOff), None);
    }
}
