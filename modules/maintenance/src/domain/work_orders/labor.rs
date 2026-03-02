//! Work order labor — add/list/remove labor entries on a work order.
//!
//! Lifecycle guard: labor cannot be modified when the WO status is
//! completed, closed, or cancelled.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use super::types::WoStatus;

// ── Domain model ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WoLabor {
    pub id: Uuid,
    pub tenant_id: String,
    pub work_order_id: Uuid,
    pub technician_ref: String,
    pub hours_decimal: Decimal,
    pub rate_minor: i64,
    pub currency: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ── Request types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddLaborRequest {
    pub tenant_id: String,
    pub technician_ref: String,
    pub hours_decimal: String,
    pub rate_minor: i64,
    pub currency: Option<String>,
    pub description: Option<String>,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum WoLaborError {
    #[error("Work order not found")]
    WoNotFound,

    #[error("Labor entry not found")]
    LaborNotFound,

    #[error("Cannot modify labor: work order status is {0}")]
    WoImmutable(WoStatus),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Lifecycle guard ───────────────────────────────────────────

const IMMUTABLE_STATUSES: &[WoStatus] =
    &[WoStatus::Completed, WoStatus::Closed, WoStatus::Cancelled];

fn check_modifiable(status: WoStatus) -> Result<(), WoLaborError> {
    if IMMUTABLE_STATUSES.contains(&status) {
        return Err(WoLaborError::WoImmutable(status));
    }
    Ok(())
}

// ── Repository ────────────────────────────────────────────────

pub struct WoLaborRepo;

impl WoLaborRepo {
    async fn wo_status(
        pool: &PgPool,
        wo_id: Uuid,
        tenant_id: &str,
    ) -> Result<WoStatus, WoLaborError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT status FROM work_orders WHERE id = $1 AND tenant_id = $2")
                .bind(wo_id)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await?;

        match row {
            Some((s,)) => {
                WoStatus::from_str_value(&s).map_err(|e| WoLaborError::Validation(e.to_string()))
            }
            None => Err(WoLaborError::WoNotFound),
        }
    }

    /// Add a labor entry to a work order.
    pub async fn add(
        pool: &PgPool,
        wo_id: Uuid,
        req: &AddLaborRequest,
    ) -> Result<WoLabor, WoLaborError> {
        if req.tenant_id.trim().is_empty() {
            return Err(WoLaborError::Validation("tenant_id is required".into()));
        }
        if req.technician_ref.trim().is_empty() {
            return Err(WoLaborError::Validation(
                "technician_ref is required".into(),
            ));
        }

        let hours: Decimal = req
            .hours_decimal
            .parse()
            .map_err(|_| WoLaborError::Validation("hours_decimal must be a valid number".into()))?;

        if hours <= Decimal::ZERO {
            return Err(WoLaborError::Validation(
                "hours_decimal must be greater than 0".into(),
            ));
        }
        if req.rate_minor < 0 {
            return Err(WoLaborError::Validation("rate_minor must be >= 0".into()));
        }

        let status = Self::wo_status(pool, wo_id, &req.tenant_id).await?;
        check_modifiable(status)?;

        let currency = req.currency.as_deref().unwrap_or("USD");

        let labor = sqlx::query_as::<_, WoLabor>(
            r#"
            INSERT INTO work_order_labor
                (tenant_id, work_order_id, technician_ref, hours_decimal,
                 rate_minor, currency, description)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(wo_id)
        .bind(req.technician_ref.trim())
        .bind(hours)
        .bind(req.rate_minor)
        .bind(currency)
        .bind(req.description.as_deref())
        .fetch_one(pool)
        .await?;

        Ok(labor)
    }

    /// List labor entries for a work order.
    pub async fn list(
        pool: &PgPool,
        wo_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<WoLabor>, WoLaborError> {
        if tenant_id.trim().is_empty() {
            return Err(WoLaborError::Validation("tenant_id is required".into()));
        }

        let _status = Self::wo_status(pool, wo_id, tenant_id).await?;

        let entries = sqlx::query_as::<_, WoLabor>(
            r#"
            SELECT * FROM work_order_labor
            WHERE work_order_id = $1 AND tenant_id = $2
            ORDER BY created_at ASC
            "#,
        )
        .bind(wo_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;

        Ok(entries)
    }

    /// Remove a labor entry from a work order.
    pub async fn remove(
        pool: &PgPool,
        wo_id: Uuid,
        labor_id: Uuid,
        tenant_id: &str,
    ) -> Result<(), WoLaborError> {
        if tenant_id.trim().is_empty() {
            return Err(WoLaborError::Validation("tenant_id is required".into()));
        }

        let status = Self::wo_status(pool, wo_id, tenant_id).await?;
        check_modifiable(status)?;

        let result = sqlx::query(
            r#"
            DELETE FROM work_order_labor
            WHERE id = $1 AND work_order_id = $2 AND tenant_id = $3
            "#,
        )
        .bind(labor_id)
        .bind(wo_id)
        .bind(tenant_id)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(WoLaborError::LaborNotFound);
        }

        Ok(())
    }
}
