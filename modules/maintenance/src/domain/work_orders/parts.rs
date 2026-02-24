//! Work order parts — add/list/remove parts on a work order.
//!
//! Lifecycle guard: parts cannot be modified when the WO status is
//! completed, closed, or cancelled.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use super::types::WoStatus;

// ── Domain model ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WoPart {
    pub id: Uuid,
    pub tenant_id: String,
    pub work_order_id: Uuid,
    pub part_description: String,
    pub part_ref: Option<String>,
    pub quantity: i32,
    pub unit_cost_minor: i64,
    pub currency: String,
    pub inventory_issue_ref: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

// ── Request types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddPartRequest {
    pub tenant_id: String,
    pub part_description: String,
    pub part_ref: Option<String>,
    pub quantity: i32,
    pub unit_cost_minor: i64,
    pub currency: Option<String>,
    pub inventory_issue_ref: Option<Uuid>,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum WoPartError {
    #[error("Work order not found")]
    WoNotFound,

    #[error("Part not found")]
    PartNotFound,

    #[error("Cannot modify parts: work order status is {0}")]
    WoImmutable(WoStatus),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Lifecycle guard ───────────────────────────────────────────

const IMMUTABLE_STATUSES: &[WoStatus] = &[
    WoStatus::Completed,
    WoStatus::Closed,
    WoStatus::Cancelled,
];

fn check_modifiable(status: WoStatus) -> Result<(), WoPartError> {
    if IMMUTABLE_STATUSES.contains(&status) {
        return Err(WoPartError::WoImmutable(status));
    }
    Ok(())
}

// ── Repository ────────────────────────────────────────────────

pub struct WoPartsRepo;

impl WoPartsRepo {
    /// Fetch the WO status for lifecycle guard checks.
    async fn wo_status(
        pool: &PgPool,
        wo_id: Uuid,
        tenant_id: &str,
    ) -> Result<WoStatus, WoPartError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT status FROM work_orders WHERE id = $1 AND tenant_id = $2",
        )
        .bind(wo_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;

        match row {
            Some((s,)) => WoStatus::from_str_value(&s)
                .map_err(|e| WoPartError::Validation(e.to_string())),
            None => Err(WoPartError::WoNotFound),
        }
    }

    /// Add a part to a work order (standalone mode — no inventory integration).
    pub async fn add(
        pool: &PgPool,
        wo_id: Uuid,
        req: &AddPartRequest,
    ) -> Result<WoPart, WoPartError> {
        if req.tenant_id.trim().is_empty() {
            return Err(WoPartError::Validation("tenant_id is required".into()));
        }
        if req.part_description.trim().is_empty() {
            return Err(WoPartError::Validation(
                "part_description is required".into(),
            ));
        }
        if req.quantity <= 0 {
            return Err(WoPartError::Validation(
                "quantity must be greater than 0".into(),
            ));
        }
        if req.unit_cost_minor < 0 {
            return Err(WoPartError::Validation(
                "unit_cost_minor must be >= 0".into(),
            ));
        }

        let status = Self::wo_status(pool, wo_id, &req.tenant_id).await?;
        check_modifiable(status)?;

        let currency = req.currency.as_deref().unwrap_or("USD");

        let part = sqlx::query_as::<_, WoPart>(
            r#"
            INSERT INTO work_order_parts
                (tenant_id, work_order_id, part_description, part_ref,
                 quantity, unit_cost_minor, currency, inventory_issue_ref)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(wo_id)
        .bind(req.part_description.trim())
        .bind(req.part_ref.as_deref())
        .bind(req.quantity)
        .bind(req.unit_cost_minor)
        .bind(currency)
        .bind(req.inventory_issue_ref)
        .fetch_one(pool)
        .await?;

        Ok(part)
    }

    /// List parts for a work order.
    pub async fn list(
        pool: &PgPool,
        wo_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<WoPart>, WoPartError> {
        if tenant_id.trim().is_empty() {
            return Err(WoPartError::Validation("tenant_id is required".into()));
        }

        // Verify WO exists for this tenant
        let _status = Self::wo_status(pool, wo_id, tenant_id).await?;

        let parts = sqlx::query_as::<_, WoPart>(
            r#"
            SELECT * FROM work_order_parts
            WHERE work_order_id = $1 AND tenant_id = $2
            ORDER BY created_at ASC
            "#,
        )
        .bind(wo_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;

        Ok(parts)
    }

    /// Remove a part from a work order.
    pub async fn remove(
        pool: &PgPool,
        wo_id: Uuid,
        part_id: Uuid,
        tenant_id: &str,
    ) -> Result<(), WoPartError> {
        if tenant_id.trim().is_empty() {
            return Err(WoPartError::Validation("tenant_id is required".into()));
        }

        let status = Self::wo_status(pool, wo_id, tenant_id).await?;
        check_modifiable(status)?;

        let result = sqlx::query(
            r#"
            DELETE FROM work_order_parts
            WHERE id = $1 AND work_order_id = $2 AND tenant_id = $3
            "#,
        )
        .bind(part_id)
        .bind(wo_id)
        .bind(tenant_id)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(WoPartError::PartNotFound);
        }

        Ok(())
    }
}
