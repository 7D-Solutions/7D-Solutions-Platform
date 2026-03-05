use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::outbox::enqueue_event;
use crate::events::{self, ProductionEventType};

// ============================================================================
// Status enum
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkOrderStatus {
    Draft,
    Released,
    Closed,
}

impl WorkOrderStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Released => "released",
            Self::Closed => "closed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(Self::Draft),
            "released" => Some(Self::Released),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkOrder {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
    pub status: String,
    pub item_id: Uuid,
    pub bom_revision_id: Uuid,
    pub routing_template_id: Option<Uuid>,
    pub planned_quantity: i32,
    pub completed_quantity: i32,
    pub planned_start: Option<DateTime<Utc>>,
    pub planned_end: Option<DateTime<Utc>>,
    pub actual_start: Option<DateTime<Utc>>,
    pub actual_end: Option<DateTime<Utc>>,
    pub material_cost_minor: i64,
    pub labor_cost_minor: i64,
    pub overhead_cost_minor: i64,
    pub correlation_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkOrder {
    pub fn parsed_status(&self) -> Option<WorkOrderStatus> {
        WorkOrderStatus::from_str(&self.status)
    }
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateWorkOrderRequest {
    pub tenant_id: String,
    pub order_number: String,
    pub item_id: Uuid,
    pub bom_revision_id: Uuid,
    pub routing_template_id: Option<Uuid>,
    pub planned_quantity: i32,
    pub planned_start: Option<DateTime<Utc>>,
    pub planned_end: Option<DateTime<Utc>>,
    pub correlation_id: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum WorkOrderError {
    #[error("Work order not found")]
    NotFound,

    #[error("Order number '{0}' already exists for tenant '{1}'")]
    DuplicateOrderNumber(String, String),

    #[error("Duplicate correlation_id — work order already exists")]
    DuplicateCorrelation,

    #[error("Invalid state transition: cannot move from '{from}' to '{to}'")]
    InvalidTransition { from: String, to: String },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Repository
// ============================================================================

pub struct WorkOrderRepo;

impl WorkOrderRepo {
    /// Create a new work order in draft status.
    /// If correlation_id is provided, duplicate requests return the existing WO.
    pub async fn create(
        pool: &PgPool,
        req: &CreateWorkOrderRequest,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<WorkOrder, WorkOrderError> {
        if req.tenant_id.trim().is_empty() {
            return Err(WorkOrderError::Validation(
                "tenant_id is required".to_string(),
            ));
        }
        if req.order_number.trim().is_empty() {
            return Err(WorkOrderError::Validation(
                "order_number is required".to_string(),
            ));
        }
        if req.planned_quantity <= 0 {
            return Err(WorkOrderError::Validation(
                "planned_quantity must be > 0".to_string(),
            ));
        }

        // Idempotency: if correlation_id provided, check for existing WO
        if let Some(ref req_corr) = req.correlation_id {
            if let Some(existing) = sqlx::query_as::<_, WorkOrder>(
                "SELECT * FROM work_orders WHERE tenant_id = $1 AND correlation_id = $2",
            )
            .bind(&req.tenant_id)
            .bind(req_corr)
            .fetch_optional(pool)
            .await?
            {
                return Ok(existing);
            }
        }

        let mut tx = pool.begin().await?;

        let wo = sqlx::query_as::<_, WorkOrder>(
            r#"
            INSERT INTO work_orders
                (tenant_id, order_number, status, item_id, bom_revision_id,
                 routing_template_id, planned_quantity, planned_start, planned_end,
                 correlation_id)
            VALUES ($1, $2, 'draft', $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(&req.order_number)
        .bind(req.item_id)
        .bind(req.bom_revision_id)
        .bind(req.routing_template_id)
        .bind(req.planned_quantity)
        .bind(req.planned_start)
        .bind(req.planned_end)
        .bind(&req.correlation_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    let msg = dbe.message().to_lowercase();
                    if msg.contains("correlation") {
                        return WorkOrderError::DuplicateCorrelation;
                    }
                    return WorkOrderError::DuplicateOrderNumber(
                        req.order_number.clone(),
                        req.tenant_id.clone(),
                    );
                }
            }
            WorkOrderError::Database(e)
        })?;

        enqueue_event(
            &mut tx,
            &req.tenant_id,
            ProductionEventType::WorkOrderCreated,
            "work_order",
            &wo.work_order_id.to_string(),
            &events::build_work_order_created_envelope(
                wo.work_order_id,
                req.tenant_id.clone(),
                req.order_number.clone(),
                req.item_id,
                req.bom_revision_id,
                req.planned_quantity,
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        tx.commit().await?;
        Ok(wo)
    }

    /// Transition: draft → released
    pub async fn release(
        pool: &PgPool,
        work_order_id: Uuid,
        tenant_id: &str,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<WorkOrder, WorkOrderError> {
        let mut tx = pool.begin().await?;

        let wo = sqlx::query_as::<_, WorkOrder>(
            "SELECT * FROM work_orders WHERE work_order_id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(work_order_id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(WorkOrderError::NotFound)?;

        let current = WorkOrderStatus::from_str(&wo.status).ok_or_else(|| {
            WorkOrderError::Validation(format!("Unknown status: {}", wo.status))
        })?;

        if current != WorkOrderStatus::Draft {
            return Err(WorkOrderError::InvalidTransition {
                from: wo.status.clone(),
                to: "released".to_string(),
            });
        }

        let updated = sqlx::query_as::<_, WorkOrder>(
            r#"
            UPDATE work_orders
            SET status = 'released', actual_start = NOW(), updated_at = NOW()
            WHERE work_order_id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(work_order_id)
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            tenant_id,
            ProductionEventType::WorkOrderReleased,
            "work_order",
            &work_order_id.to_string(),
            &events::build_work_order_released_envelope(
                work_order_id,
                tenant_id.to_string(),
                updated.order_number.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        tx.commit().await?;
        Ok(updated)
    }

    /// Transition: released → closed
    pub async fn close(
        pool: &PgPool,
        work_order_id: Uuid,
        tenant_id: &str,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<WorkOrder, WorkOrderError> {
        let mut tx = pool.begin().await?;

        let wo = sqlx::query_as::<_, WorkOrder>(
            "SELECT * FROM work_orders WHERE work_order_id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(work_order_id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(WorkOrderError::NotFound)?;

        let current = WorkOrderStatus::from_str(&wo.status).ok_or_else(|| {
            WorkOrderError::Validation(format!("Unknown status: {}", wo.status))
        })?;

        if current != WorkOrderStatus::Released {
            return Err(WorkOrderError::InvalidTransition {
                from: wo.status.clone(),
                to: "closed".to_string(),
            });
        }

        let updated = sqlx::query_as::<_, WorkOrder>(
            r#"
            UPDATE work_orders
            SET status = 'closed', actual_end = NOW(), updated_at = NOW()
            WHERE work_order_id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(work_order_id)
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            tenant_id,
            ProductionEventType::WorkOrderClosed,
            "work_order",
            &work_order_id.to_string(),
            &events::build_work_order_closed_envelope(
                work_order_id,
                tenant_id.to_string(),
                updated.order_number.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        tx.commit().await?;
        Ok(updated)
    }

    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<WorkOrder>, WorkOrderError> {
        sqlx::query_as::<_, WorkOrder>(
            "SELECT * FROM work_orders WHERE work_order_id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(WorkOrderError::Database)
    }
}
