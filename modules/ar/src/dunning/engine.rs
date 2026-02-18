//! Dunning engine — DB-transactional init and transition functions.
//!
//! These functions apply state machine transitions atomically with outbox events.
//! The guard logic lives in the parent module (`is_valid_transition`).

use crate::events::{
    build_dunning_state_changed_envelope, build_invoice_suspended_envelope,
    DunningStateChangedPayload, InvoiceSuspendedPayload,
    EVENT_TYPE_DUNNING_STATE_CHANGED, EVENT_TYPE_INVOICE_SUSPENDED,
};
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    is_valid_transition, DunningError, DunningStateRow, DunningStateValue,
    InitDunningRequest, InitDunningResult, TransitionDunningRequest, TransitionDunningResult,
};

/// Initialize dunning for an invoice (creates the Pending state record).
///
/// **Idempotency**: duplicate `dunning_id` or duplicate `(app_id, invoice_id)` returns
/// `AlreadyExists` without error. Uses `INSERT ON CONFLICT DO NOTHING` to avoid two
/// pre-flight SELECT checks — unique constraints enforce idempotency atomically.
///
/// **Atomicity**: dunning record + outbox event (LIFECYCLE) are inserted in
/// a single transaction.
pub async fn init_dunning(
    pool: &PgPool,
    req: InitDunningRequest,
) -> Result<InitDunningResult, DunningError> {
    let mut tx = pool.begin().await?;
    let now = Utc::now();

    // Pre-generate outbox_event_id so it can be set in the INSERT itself,
    // eliminating the trailing UPDATE that previously correlated the two rows.
    let outbox_event_id = Uuid::new_v4();

    // 1. Insert the initial Pending state record.
    //    ON CONFLICT DO NOTHING handles both idempotency cases atomically:
    //      - duplicate dunning_id (UNIQUE on dunning_id)
    //      - duplicate (app_id, invoice_id) (UNIQUE on ar_dunning_states_unique_invoice)
    //    This replaces two pre-flight SELECT checks with a single INSERT attempt.
    let maybe_dunning_row_id: Option<i32> = sqlx::query_scalar(
        r#"
        INSERT INTO ar_dunning_states (
            dunning_id, app_id, invoice_id, customer_id,
            state, version, attempt_count, next_attempt_at,
            outbox_event_id, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, 'pending', 1, 0, $5, $6, $7, $7)
        ON CONFLICT DO NOTHING
        RETURNING id
        "#,
    )
    .bind(req.dunning_id)
    .bind(&req.app_id)
    .bind(req.invoice_id)
    .bind(&req.customer_id)
    .bind(req.next_attempt_at)
    .bind(outbox_event_id)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;

    let dunning_row_id = match maybe_dunning_row_id {
        Some(id) => id,
        None => {
            // A unique constraint was violated — record already exists.
            // Rollback and return AlreadyExists with the existing row id.
            tx.rollback().await?;
            let existing_row_id: i32 = sqlx::query_scalar(
                r#"
                SELECT id FROM ar_dunning_states
                WHERE dunning_id = $1 OR (app_id = $2 AND invoice_id = $3)
                LIMIT 1
                "#,
            )
            .bind(req.dunning_id)
            .bind(&req.app_id)
            .bind(req.invoice_id)
            .fetch_one(pool)
            .await
            .map_err(|e| DunningError::DatabaseError(e.to_string()))?;
            return Ok(InitDunningResult::AlreadyExists { existing_row_id });
        }
    };

    // 2. Build and enqueue the outbox event (LIFECYCLE)
    let payload = DunningStateChangedPayload {
        tenant_id: req.app_id.clone(),
        invoice_id: req.invoice_id.to_string(),
        customer_id: req.customer_id.clone(),
        from_state: None,
        to_state: crate::events::DunningState::Pending,
        reason: "dunning_initialized".to_string(),
        attempt_number: 0,
        next_retry_at: req.next_attempt_at,
        transitioned_at: now,
    };

    let envelope = build_dunning_state_changed_envelope(
        outbox_event_id,
        req.app_id.clone(),
        req.correlation_id.clone(),
        req.causation_id.clone(),
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
    .bind(req.dunning_id.to_string())
    .bind(payload_json)
    .bind(&req.app_id)
    .bind(&envelope.schema_version)
    .bind(now)
    .bind(&req.correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // outbox_event_id was already set in the INSERT above — no trailing UPDATE needed.

    tx.commit().await?;

    Ok(InitDunningResult::Initialized {
        dunning_row_id,
        dunning_id: req.dunning_id,
    })
}

/// Transition a dunning record to a new state.
///
/// **Guard**: validates the from → to transition before touching the DB.
/// **Atomic**: state UPDATE + outbox INSERT in a single transaction.
/// **Race-safe**: uses SELECT FOR UPDATE to prevent concurrent transitions
/// on the same dunning record.
pub async fn transition_dunning(
    pool: &PgPool,
    req: TransitionDunningRequest,
) -> Result<TransitionDunningResult, DunningError> {
    let mut tx = pool.begin().await?;
    let now = Utc::now();

    // 1. Lock the dunning record for update (race-safe)
    let row: Option<DunningStateRow> = sqlx::query_as(
        r#"
        SELECT id, dunning_id, state, version, attempt_count, customer_id
        FROM ar_dunning_states
        WHERE app_id = $1 AND invoice_id = $2
        FOR UPDATE
        "#,
    )
    .bind(&req.app_id)
    .bind(req.invoice_id)
    .fetch_optional(&mut *tx)
    .await?;

    let row = match row {
        Some(r) => r,
        None => {
            tx.rollback().await?;
            return Err(DunningError::DunningNotFound {
                invoice_id: req.invoice_id,
                app_id: req.app_id,
            });
        }
    };

    let from_state = DunningStateValue::from_str(&row.state).ok_or_else(|| {
        DunningError::DatabaseError(format!("Unknown dunning state in DB: {}", row.state))
    })?;

    // 2. Guard: reject terminal → anything transitions
    if from_state.is_terminal() {
        tx.rollback().await?;
        return Err(DunningError::TerminalState {
            state: row.state.clone(),
        });
    }

    // 3. Guard: validate the specific transition
    if !is_valid_transition(&from_state, &req.to_state) {
        tx.rollback().await?;
        return Err(DunningError::IllegalTransition {
            from_state: from_state.as_str().to_string(),
            to_state: req.to_state.as_str().to_string(),
        });
    }

    // 4. Increment attempt_count when moving to an attempt-based state
    let new_attempt_count = match &req.to_state {
        DunningStateValue::Warned | DunningStateValue::Escalated | DunningStateValue::Suspended => {
            row.attempt_count + 1
        }
        _ => row.attempt_count,
    };
    let new_version = row.version + 1;

    // Pre-generate outbox_event_id so it can be merged into the UPDATE itself,
    // eliminating the trailing UPDATE that previously correlated the two rows.
    let outbox_event_id = Uuid::new_v4();

    // 5. Apply the transition and set outbox_event_id in one UPDATE (optimistic lock: version must match)
    let rows_updated = sqlx::query(
        r#"
        UPDATE ar_dunning_states
        SET
            state           = $1,
            version         = version + 1,
            attempt_count   = $2,
            next_attempt_at = $3,
            last_error      = $4,
            updated_at      = $5,
            outbox_event_id = $9
        WHERE
            app_id      = $6
            AND invoice_id  = $7
            AND version     = $8
        "#,
    )
    .bind(req.to_state.as_str())
    .bind(new_attempt_count)
    .bind(&req.next_attempt_at)
    .bind(&req.last_error)
    .bind(now)
    .bind(&req.app_id)
    .bind(req.invoice_id)
    .bind(row.version)
    .bind(outbox_event_id)
    .execute(&mut *tx)
    .await?;

    if rows_updated.rows_affected() == 0 {
        tx.rollback().await?;
        return Err(DunningError::ConcurrentModification {
            invoice_id: req.invoice_id,
        });
    }

    // 6. Build and enqueue the outbox event (LIFECYCLE)
    let payload = DunningStateChangedPayload {
        tenant_id: req.app_id.clone(),
        invoice_id: req.invoice_id.to_string(),
        customer_id: row.customer_id.clone(),
        from_state: Some(from_state.to_event_state()),
        to_state: req.to_state.to_event_state(),
        reason: req.reason.clone(),
        attempt_number: new_attempt_count,
        next_retry_at: req.next_attempt_at,
        transitioned_at: now,
    };

    let envelope = build_dunning_state_changed_envelope(
        outbox_event_id,
        req.app_id.clone(),
        req.correlation_id.clone(),
        req.causation_id.clone(),
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
    .bind(&req.app_id)
    .bind(&envelope.schema_version)
    .bind(now)
    .bind(&req.correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // 6b. If transitioning to Suspended, also emit ar.invoice_suspended
    //     This cross-module signal lets subscriptions apply suspension.
    if req.to_state == DunningStateValue::Suspended {
        let suspended_event_id = Uuid::new_v4();
        let suspended_payload = InvoiceSuspendedPayload {
            tenant_id: req.app_id.clone(),
            invoice_id: req.invoice_id.to_string(),
            customer_id: row.customer_id.clone(),
            outstanding_minor: 0, // AR does not carry balance in dunning row; downstream can look it up
            currency: String::new(),
            dunning_attempt: new_attempt_count,
            reason: req.reason.clone(),
            grace_period_ends_at: None,
            suspended_at: now,
        };

        let suspended_envelope = build_invoice_suspended_envelope(
            suspended_event_id,
            req.app_id.clone(),
            req.correlation_id.clone(),
            Some(outbox_event_id.to_string()), // causation: the dunning_state_changed event
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
        .bind(req.invoice_id.to_string())
        .bind(suspended_json)
        .bind(&req.app_id)
        .bind(&suspended_envelope.schema_version)
        .bind(now)
        .bind(&req.correlation_id)
        .bind(&outbox_event_id.to_string())
        .execute(&mut *tx)
        .await?;
    }

    // outbox_event_id was already set in the UPDATE above — no trailing UPDATE needed.

    tx.commit().await?;

    Ok(TransitionDunningResult::Transitioned {
        dunning_row_id: row.id,
        from_state,
        to_state: req.to_state,
        new_version,
        new_attempt_count,
    })
}
