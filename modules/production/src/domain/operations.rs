use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::outbox::enqueue_event;
use crate::events::{self, ProductionEventType};

// ============================================================================
// Status enum
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Pending,
    InProgress,
    Completed,
}

impl OperationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct OperationInstance {
    pub operation_id: Uuid,
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub routing_step_id: Option<Uuid>,
    pub sequence_number: i32,
    pub workcenter_id: Uuid,
    pub operation_name: String,
    pub status: String,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub labor_minutes: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum OperationError {
    #[error("Operation not found")]
    NotFound,

    #[error("Work order not found")]
    WorkOrderNotFound,

    #[error("Work order is not in 'released' status")]
    WorkOrderNotReleased,

    #[error("No routing template assigned to work order")]
    NoRoutingTemplate,

    #[error("Operations already initialized for this work order")]
    AlreadyInitialized,

    #[error("Invalid state transition: cannot move from '{from}' to '{to}'")]
    InvalidTransition { from: String, to: String },

    #[error("Predecessor operation (seq {0}) is not completed")]
    PredecessorNotComplete(i32),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Repository
// ============================================================================

pub struct OperationRepo;

impl OperationRepo {
    /// Materialize operation instances from the routing template's steps.
    pub async fn initialize(
        pool: &PgPool,
        work_order_id: Uuid,
        tenant_id: &str,
        _correlation_id: &str,
        _causation_id: Option<&str>,
    ) -> Result<Vec<OperationInstance>, OperationError> {
        let mut tx = pool.begin().await?;

        // Verify work order exists, belongs to tenant, and is released
        let wo = sqlx::query_as::<_, (String, Option<Uuid>)>(
            "SELECT status, routing_template_id FROM work_orders WHERE work_order_id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(work_order_id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(OperationError::WorkOrderNotFound)?;

        if wo.0 != "released" {
            return Err(OperationError::WorkOrderNotReleased);
        }

        let routing_template_id = wo.1.ok_or(OperationError::NoRoutingTemplate)?;

        // Check no operations already exist
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM operations WHERE work_order_id = $1",
        )
        .bind(work_order_id)
        .fetch_one(&mut *tx)
        .await?;

        if count.0 > 0 {
            return Err(OperationError::AlreadyInitialized);
        }

        // Fetch routing steps ordered by sequence
        let steps = sqlx::query_as::<_, (Uuid, i32, Uuid, String, Option<String>, Option<i32>, Option<i32>, bool)>(
            r#"
            SELECT routing_step_id, sequence_number, workcenter_id, operation_name,
                   description, setup_time_minutes, run_time_minutes, is_required
            FROM routing_steps
            WHERE routing_template_id = $1
            ORDER BY sequence_number
            "#,
        )
        .bind(routing_template_id)
        .fetch_all(&mut *tx)
        .await?;

        let ops = if steps.is_empty() {
            Vec::new()
        } else {
            let routing_step_ids: Vec<Uuid> = steps.iter().map(|s| s.0).collect();
            let sequence_numbers: Vec<i32> = steps.iter().map(|s| s.1).collect();
            let workcenter_ids: Vec<Uuid> = steps.iter().map(|s| s.2).collect();
            let operation_names: Vec<&str> = steps.iter().map(|s| s.3.as_str()).collect();

            let mut rows = sqlx::query_as::<_, OperationInstance>(
                r#"
                INSERT INTO operations
                    (work_order_id, tenant_id, routing_step_id, sequence_number,
                     workcenter_id, operation_name, status)
                SELECT $1, $2,
                    UNNEST($3::UUID[]),
                    UNNEST($4::INT[]),
                    UNNEST($5::UUID[]),
                    UNNEST($6::TEXT[]),
                    'pending'
                RETURNING *
                "#,
            )
            .bind(work_order_id)
            .bind(tenant_id)
            .bind(&routing_step_ids)
            .bind(&sequence_numbers)
            .bind(&workcenter_ids)
            .bind(&operation_names)
            .fetch_all(&mut *tx)
            .await?;

            rows.sort_by_key(|o| o.sequence_number);
            rows
        };

        tx.commit().await?;
        Ok(ops)
    }

    /// Start an operation: pending → in_progress.
    /// Enforces that all prior required operations are completed.
    pub async fn start(
        pool: &PgPool,
        work_order_id: Uuid,
        operation_id: Uuid,
        tenant_id: &str,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<OperationInstance, OperationError> {
        let mut tx = pool.begin().await?;

        let op = sqlx::query_as::<_, OperationInstance>(
            "SELECT * FROM operations WHERE operation_id = $1 AND work_order_id = $2 AND tenant_id = $3 FOR UPDATE",
        )
        .bind(operation_id)
        .bind(work_order_id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(OperationError::NotFound)?;

        let current = OperationStatus::from_str(&op.status).unwrap_or(OperationStatus::Pending);
        if current != OperationStatus::Pending {
            return Err(OperationError::InvalidTransition {
                from: op.status.clone(),
                to: "in_progress".to_string(),
            });
        }

        // Enforce ordering: all prior required operations must be completed.
        // A "required" operation is one sourced from a routing step with is_required = true.
        let blocking = sqlx::query_as::<_, (i32,)>(
            r#"
            SELECT o.sequence_number
            FROM operations o
            JOIN routing_steps rs ON rs.routing_step_id = o.routing_step_id
            WHERE o.work_order_id = $1
              AND o.sequence_number < $2
              AND o.status != 'completed'
              AND rs.is_required = TRUE
            ORDER BY o.sequence_number
            LIMIT 1
            "#,
        )
        .bind(work_order_id)
        .bind(op.sequence_number)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some((seq,)) = blocking {
            return Err(OperationError::PredecessorNotComplete(seq));
        }

        let updated = sqlx::query_as::<_, OperationInstance>(
            r#"
            UPDATE operations
            SET status = 'in_progress', started_at = NOW(), updated_at = NOW()
            WHERE operation_id = $1
            RETURNING *
            "#,
        )
        .bind(operation_id)
        .fetch_one(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            tenant_id,
            ProductionEventType::OperationStarted,
            "operation",
            &operation_id.to_string(),
            &events::build_operation_started_envelope(
                operation_id,
                work_order_id,
                tenant_id.to_string(),
                updated.operation_name.clone(),
                updated.sequence_number,
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

    /// Complete an operation: in_progress → completed.
    pub async fn complete(
        pool: &PgPool,
        work_order_id: Uuid,
        operation_id: Uuid,
        tenant_id: &str,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<OperationInstance, OperationError> {
        let mut tx = pool.begin().await?;

        let op = sqlx::query_as::<_, OperationInstance>(
            "SELECT * FROM operations WHERE operation_id = $1 AND work_order_id = $2 AND tenant_id = $3 FOR UPDATE",
        )
        .bind(operation_id)
        .bind(work_order_id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(OperationError::NotFound)?;

        let current = OperationStatus::from_str(&op.status).unwrap_or(OperationStatus::Pending);
        if current != OperationStatus::InProgress {
            return Err(OperationError::InvalidTransition {
                from: op.status.clone(),
                to: "completed".to_string(),
            });
        }

        let updated = sqlx::query_as::<_, OperationInstance>(
            r#"
            UPDATE operations
            SET status = 'completed', completed_at = NOW(), updated_at = NOW()
            WHERE operation_id = $1
            RETURNING *
            "#,
        )
        .bind(operation_id)
        .fetch_one(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            tenant_id,
            ProductionEventType::OperationCompleted,
            "operation",
            &operation_id.to_string(),
            &events::build_operation_completed_envelope(
                operation_id,
                work_order_id,
                tenant_id.to_string(),
                updated.operation_name.clone(),
                updated.sequence_number,
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

    /// List all operations for a work order, ordered by sequence.
    pub async fn list(
        pool: &PgPool,
        work_order_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<OperationInstance>, OperationError> {
        sqlx::query_as::<_, OperationInstance>(
            "SELECT * FROM operations WHERE work_order_id = $1 AND tenant_id = $2 ORDER BY sequence_number",
        )
        .bind(work_order_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
        .map_err(OperationError::Database)
    }
}
