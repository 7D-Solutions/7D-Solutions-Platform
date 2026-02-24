//! Work order service — Guard→Mutation→Outbox for all WO lifecycle operations.
//!
//! Invariants:
//! - wo_number is tenant-scoped and sequential (wo_counters + SELECT FOR UPDATE)
//! - Every status transition is validated by state_machine + guards
//! - Every mutation writes its event to the outbox atomically in the same tx
//! - All queries filter by tenant_id

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use super::guards::{run_guards, GuardError, TransitionContext};
use super::state_machine::{validate_transition, TransitionError};
use super::types::{Priority, WoStatus, WoType};
use crate::outbox;

// ── Domain model ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkOrder {
    pub id: Uuid,
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub plan_assignment_id: Option<Uuid>,
    pub wo_number: String,
    pub title: String,
    pub description: Option<String>,
    #[sqlx(try_from = "String")]
    pub wo_type: WoType,
    #[sqlx(try_from = "String")]
    pub priority: Priority,
    #[sqlx(try_from = "String")]
    pub status: WoStatus,
    pub assigned_to: Option<String>,
    pub scheduled_date: Option<NaiveDate>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub checklist: Option<serde_json::Value>,
    pub downtime_minutes: Option<i32>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Request / query types ─────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateWorkOrderRequest {
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub plan_assignment_id: Option<Uuid>,
    pub title: String,
    pub description: Option<String>,
    pub wo_type: String,
    pub priority: Option<String>,
    pub assigned_to: Option<String>,
    pub scheduled_date: Option<NaiveDate>,
    pub checklist: Option<serde_json::Value>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TransitionRequest {
    pub tenant_id: String,
    pub status: String,
    pub completed_at: Option<DateTime<Utc>>,
    pub downtime_minutes: Option<i32>,
    pub closed_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListWorkOrdersQuery {
    pub tenant_id: String,
    pub asset_id: Option<Uuid>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum WoError {
    #[error("Work order not found")]
    NotFound,

    #[error("Asset not found")]
    AssetNotFound,

    #[error("Plan assignment not found")]
    AssignmentNotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Transition error: {0}")]
    Transition(#[from] TransitionError),

    #[error("Guard error: {0}")]
    Guard(#[from] GuardError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Repository ────────────────────────────────────────────────

pub struct WorkOrderRepo;

impl WorkOrderRepo {
    /// Allocate the next sequential WO number for a tenant.
    /// Uses SELECT FOR UPDATE for race-safety.
    async fn next_wo_number(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
    ) -> Result<String, sqlx::Error> {
        // Upsert the counter row (first WO for tenant creates it)
        let row: (i64,) = sqlx::query_as(
            r#"
            INSERT INTO wo_counters (tenant_id, next_number)
            VALUES ($1, 2)
            ON CONFLICT (tenant_id) DO UPDATE
                SET next_number = wo_counters.next_number + 1
            RETURNING next_number - 1
            "#,
        )
        .bind(tenant_id)
        .fetch_one(&mut **tx)
        .await?;

        Ok(format!("WO-{:06}", row.0))
    }

    /// Create a work order (ad-hoc or from plan assignment).
    pub async fn create(
        pool: &PgPool,
        req: &CreateWorkOrderRequest,
    ) -> Result<WorkOrder, WoError> {
        // ── Guards ──
        if req.tenant_id.trim().is_empty() {
            return Err(WoError::Validation("tenant_id is required".into()));
        }
        if req.title.trim().is_empty() {
            return Err(WoError::Validation("title is required".into()));
        }
        WoType::from_str_value(&req.wo_type)
            .map_err(|e| WoError::Validation(e.to_string()))?;
        let priority_str = req.priority.as_deref().unwrap_or("medium");
        Priority::from_str_value(priority_str)
            .map_err(|e| WoError::Validation(e.to_string()))?;

        let mut tx = pool.begin().await?;

        // Verify asset exists
        let asset_exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM maintainable_assets WHERE id = $1 AND tenant_id = $2",
        )
        .bind(req.asset_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?;
        if asset_exists.is_none() {
            return Err(WoError::AssetNotFound);
        }

        // Verify plan assignment if provided
        if let Some(pa_id) = req.plan_assignment_id {
            let pa_exists: Option<(Uuid,)> = sqlx::query_as(
                "SELECT id FROM maintenance_plan_assignments WHERE id = $1 AND tenant_id = $2",
            )
            .bind(pa_id)
            .bind(&req.tenant_id)
            .fetch_optional(&mut *tx)
            .await?;
            if pa_exists.is_none() {
                return Err(WoError::AssignmentNotFound);
            }
        }

        // ── Mutation ──
        let wo_number = Self::next_wo_number(&mut tx, &req.tenant_id).await?;
        let id = Uuid::new_v4();

        let wo = sqlx::query_as::<_, WorkOrder>(
            r#"
            INSERT INTO work_orders
                (id, tenant_id, asset_id, plan_assignment_id, wo_number,
                 title, description, wo_type, priority, assigned_to,
                 scheduled_date, checklist, notes)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.asset_id)
        .bind(req.plan_assignment_id)
        .bind(&wo_number)
        .bind(req.title.trim())
        .bind(req.description.as_deref())
        .bind(&req.wo_type)
        .bind(priority_str)
        .bind(req.assigned_to.as_deref())
        .bind(req.scheduled_date)
        .bind(&req.checklist)
        .bind(req.notes.as_deref())
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox ──
        let event_payload = serde_json::json!({
            "work_order_id": id,
            "tenant_id": &req.tenant_id,
            "asset_id": req.asset_id,
            "wo_number": &wo_number,
            "wo_type": &req.wo_type,
            "priority": priority_str,
            "plan_assignment_id": req.plan_assignment_id,
        });
        outbox::enqueue_event_tx(
            &mut tx,
            Uuid::new_v4(),
            "maintenance.work_order.created",
            "work_order",
            &id.to_string(),
            &event_payload,
        )
        .await?;

        tx.commit().await?;
        Ok(wo)
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
            WoStatus::Completed => "maintenance.work_order.completed",
            WoStatus::Closed => "maintenance.work_order.closed",
            WoStatus::Cancelled => "maintenance.work_order.cancelled",
            _ => "maintenance.work_order.status_changed",
        };

        let event_payload = serde_json::json!({
            "work_order_id": wo_id,
            "tenant_id": &req.tenant_id,
            "wo_number": &wo.wo_number,
            "from_status": current.status.as_str(),
            "to_status": target.as_str(),
        });
        outbox::enqueue_event_tx(
            &mut tx,
            Uuid::new_v4(),
            event_type,
            "work_order",
            &wo_id.to_string(),
            &event_payload,
        )
        .await?;

        tx.commit().await?;
        Ok(wo)
    }

    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<WorkOrder>, WoError> {
        sqlx::query_as::<_, WorkOrder>(
            "SELECT * FROM work_orders WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(WoError::Database)
    }

    pub async fn list(
        pool: &PgPool,
        q: &ListWorkOrdersQuery,
    ) -> Result<Vec<WorkOrder>, WoError> {
        if q.tenant_id.trim().is_empty() {
            return Err(WoError::Validation("tenant_id is required".into()));
        }
        let limit = q.limit.unwrap_or(50).min(100).max(1);
        let offset = q.offset.unwrap_or(0).max(0);

        if let Some(ref s) = q.status {
            WoStatus::from_str_value(s)
                .map_err(|e| WoError::Validation(e.to_string()))?;
        }

        sqlx::query_as::<_, WorkOrder>(
            r#"
            SELECT * FROM work_orders
            WHERE tenant_id = $1
              AND ($2::UUID IS NULL OR asset_id = $2)
              AND ($3::TEXT IS NULL OR status = $3)
            ORDER BY created_at DESC
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(&q.tenant_id)
        .bind(q.asset_id)
        .bind(q.status.as_deref())
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(WoError::Database)
    }
}
