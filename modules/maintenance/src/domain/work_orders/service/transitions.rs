//! Work order status transitions — guard enforcement, cost computation, and event emission.
//!
//! Invariants:
//! - Every status transition is validated by state_machine + guards
//! - Completed events include GL cost payload for downstream journal entries
//! - Every mutation writes its event to the outbox atomically in the same tx

use sqlx::PgPool;
use uuid::Uuid;

use super::super::guards::{run_guards, TransitionContext};
use super::super::state_machine::validate_transition;
use super::super::types::WoStatus;
use super::core::{TransitionRequest, WoError, WorkOrder, WorkOrderRepo};
use crate::events::{envelope, subjects};
use crate::outbox;

// ── Cost payload for GL integration ───────────────────────────

/// Cost totals computed at WO completion, embedded in the completed event.
/// Downstream GL can post journal entries deterministically from this alone.
struct CostPayload {
    total_parts_minor: i64,
    total_labor_minor: i64,
    currency: String,
    fixed_asset_ref: Option<Uuid>,
}

impl WorkOrderRepo {
    /// Compute cost totals for a completed work order within the same tx.
    ///
    /// Parts total: SUM(quantity * unit_cost_minor)
    /// Labor total: SUM(ROUND(hours_decimal * rate_minor))
    /// Currency: taken from first cost entry, or "USD" if no entries.
    /// fixed_asset_ref: from the linked maintainable_asset.
    async fn compute_cost_payload(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        wo_id: Uuid,
        tenant_id: &str,
        asset_id: Uuid,
    ) -> Result<CostPayload, sqlx::Error> {
        // Parts total + currency
        // SUM of BIGINT returns NUMERIC; cast to BIGINT for Rust i64.
        let parts_row: (i64, Option<String>) = sqlx::query_as(
            r#"
            SELECT COALESCE(SUM(quantity::bigint * unit_cost_minor), 0)::bigint,
                   MIN(currency)
            FROM work_order_parts
            WHERE work_order_id = $1 AND tenant_id = $2
            "#,
        )
        .bind(wo_id)
        .bind(tenant_id)
        .fetch_one(&mut **tx)
        .await?;

        // Labor total + currency
        // hours_decimal is NUMERIC(8,2), rate_minor is BIGINT.
        // ROUND produces NUMERIC; cast final SUM to BIGINT for Rust i64.
        let labor_row: (i64, Option<String>) = sqlx::query_as(
            r#"
            SELECT COALESCE(SUM(ROUND(hours_decimal * rate_minor))::bigint, 0),
                   MIN(currency)
            FROM work_order_labor
            WHERE work_order_id = $1 AND tenant_id = $2
            "#,
        )
        .bind(wo_id)
        .bind(tenant_id)
        .fetch_one(&mut **tx)
        .await?;

        // Pick currency from first available cost entry, default to "USD"
        let currency = parts_row.1.or(labor_row.1).unwrap_or_else(|| "USD".into());

        // Fetch fixed_asset_ref from the linked asset
        let asset_row: Option<(Option<Uuid>,)> = sqlx::query_as(
            "SELECT fixed_asset_ref FROM maintainable_assets WHERE id = $1 AND tenant_id = $2",
        )
        .bind(asset_id)
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await?;

        let fixed_asset_ref = asset_row.and_then(|r| r.0);

        Ok(CostPayload {
            total_parts_minor: parts_row.0,
            total_labor_minor: labor_row.0,
            currency,
            fixed_asset_ref,
        })
    }

    /// Transition a work order's status with guard enforcement.
    pub async fn transition(
        pool: &PgPool,
        wo_id: Uuid,
        req: &TransitionRequest,
    ) -> Result<WorkOrder, WoError> {
        if req.tenant_id.trim().is_empty() {
            return Err(WoError::Validation("tenant_id is required".into()));
        }
        let target = WoStatus::from_str_value(&req.status)
            .map_err(|e| WoError::Validation(e.to_string()))?;

        let mut tx = pool.begin().await?;

        // Fetch current WO (row-level lock via FOR UPDATE)
        let current = sqlx::query_as::<_, WorkOrder>(
            "SELECT * FROM work_orders WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(wo_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(WoError::NotFound)?;

        // ── Guard: state machine ──
        validate_transition(current.status, target)?;

        // ── Guard: field-level ──
        let ctx = TransitionContext {
            completed_at: req.completed_at,
            downtime_minutes: req.downtime_minutes,
            closed_at: req.closed_at,
        };
        run_guards(target, &ctx)?;

        // ── Mutation ──
        let wo = sqlx::query_as::<_, WorkOrder>(
            r#"
            UPDATE work_orders SET
                status           = $3,
                started_at       = CASE WHEN $3 = 'in_progress' AND started_at IS NULL
                                        THEN NOW() ELSE started_at END,
                completed_at     = COALESCE($4, completed_at),
                downtime_minutes = COALESCE($5, downtime_minutes),
                closed_at        = COALESCE($6, closed_at),
                notes            = COALESCE($7, notes),
                updated_at       = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(wo_id)
        .bind(&req.tenant_id)
        .bind(target.as_str())
        .bind(req.completed_at)
        .bind(req.downtime_minutes)
        .bind(req.closed_at)
        .bind(req.notes.as_deref())
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox: pick event type based on target status ──
        let event_type = match target {
            WoStatus::Completed => subjects::WO_COMPLETED,
            WoStatus::Closed => subjects::WO_CLOSED,
            WoStatus::Cancelled => subjects::WO_CANCELLED,
            _ => subjects::WO_STATUS_CHANGED,
        };

        // ── Cost payload for completed events (GL integration seam) ──
        let event_payload = if target == WoStatus::Completed {
            let cost =
                Self::compute_cost_payload(&mut tx, wo_id, &req.tenant_id, wo.asset_id).await?;
            serde_json::json!({
                "work_order_id": wo_id,
                "tenant_id": &req.tenant_id,
                "wo_number": &wo.wo_number,
                "from_status": current.status.as_str(),
                "to_status": target.as_str(),
                "total_parts_minor": cost.total_parts_minor,
                "total_labor_minor": cost.total_labor_minor,
                "currency": cost.currency,
                "fixed_asset_ref": cost.fixed_asset_ref,
            })
        } else {
            serde_json::json!({
                "work_order_id": wo_id,
                "tenant_id": &req.tenant_id,
                "wo_number": &wo.wo_number,
                "from_status": current.status.as_str(),
                "to_status": target.as_str(),
            })
        };
        let event_id = Uuid::new_v4();
        let env = envelope::create_envelope(
            event_id,
            req.tenant_id.clone(),
            event_type.to_string(),
            event_payload,
        );
        let env_json = envelope::validate_envelope(&env)
            .map_err(|e| WoError::Validation(format!("envelope: {}", e)))?;
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            event_type,
            "work_order",
            &wo_id.to_string(),
            &env_json,
        )
        .await?;

        tx.commit().await?;
        Ok(wo)
    }
}
