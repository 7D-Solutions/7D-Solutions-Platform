use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::numbering_client::NumberingClient;
use crate::domain::operations::OperationInstance;
use crate::domain::outbox::enqueue_event;
use crate::domain::time_entries::TimeEntry;
use crate::events::{self, ProductionEventType};
use platform_sdk::VerifiedClaims;

// ============================================================================
// Status enum
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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
// Derived status — computed at query time from operations aggregate
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DerivedStatus {
    NotStarted,
    InProgress,
    Complete,
}

impl TryFrom<String> for DerivedStatus {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "not_started" => Ok(Self::NotStarted),
            "in_progress" => Ok(Self::InProgress),
            "complete" => Ok(Self::Complete),
            other => Err(format!("Unknown derived_status: {}", other)),
        }
    }
}

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct WorkOrder {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
    pub status: String,
    pub item_id: Uuid,
    pub bom_revision_id: Option<Uuid>,
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

/// Work order with derived_status computed from operations at query time.
/// Returned by GET /work-orders/:id and GET /work-orders.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct WorkOrderResponse {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
    pub status: String,
    pub item_id: Uuid,
    pub bom_revision_id: Option<Uuid>,
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
    #[sqlx(try_from = "String")]
    pub derived_status: DerivedStatus,
}

/// Work order returned by the batch endpoint.  All WO fields are present;
/// `operations` and `time_entries` are populated only when requested via
/// `?include=operations` / `?include=time_entries`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkOrderWithIncludes {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
    pub status: String,
    pub item_id: Uuid,
    pub bom_revision_id: Option<Uuid>,
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
    pub derived_status: DerivedStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operations: Option<Vec<OperationInstance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_entries: Option<Vec<TimeEntry>>,
}

impl From<WorkOrderResponse> for WorkOrderWithIncludes {
    fn from(wo: WorkOrderResponse) -> Self {
        Self {
            work_order_id: wo.work_order_id,
            tenant_id: wo.tenant_id,
            order_number: wo.order_number,
            status: wo.status,
            item_id: wo.item_id,
            bom_revision_id: wo.bom_revision_id,
            routing_template_id: wo.routing_template_id,
            planned_quantity: wo.planned_quantity,
            completed_quantity: wo.completed_quantity,
            planned_start: wo.planned_start,
            planned_end: wo.planned_end,
            actual_start: wo.actual_start,
            actual_end: wo.actual_end,
            material_cost_minor: wo.material_cost_minor,
            labor_cost_minor: wo.labor_cost_minor,
            overhead_cost_minor: wo.overhead_cost_minor,
            correlation_id: wo.correlation_id,
            created_at: wo.created_at,
            updated_at: wo.updated_at,
            derived_status: wo.derived_status,
            operations: None,
            time_entries: None,
        }
    }
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize, ToSchema)]
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

/// Request body for `POST /api/production/work-orders/create`.
///
/// Allocates a WO number from the Numbering service, then creates the work
/// order in a single call.  `bom_revision_id` and `routing_template_id` are
/// optional; omit them to create a skeleton WO with a number only.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CompositeCreateWorkOrderRequest {
    /// Filled automatically from the JWT tenant claim — do not send in body.
    #[serde(default)]
    pub tenant_id: String,
    pub item_id: Uuid,
    pub bom_revision_id: Option<Uuid>,
    pub routing_template_id: Option<Uuid>,
    pub planned_quantity: i32,
    pub planned_start: Option<DateTime<Utc>>,
    pub planned_end: Option<DateTime<Utc>>,
    /// Idempotency key forwarded to the Numbering service.  Re-sending the
    /// same key returns the previously allocated WO number (no duplicate
    /// created).
    pub idempotency_key: String,
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

    #[error("Numbering service error: {0}")]
    NumberingService(String),

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
                Some(req.bom_revision_id),
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

    /// Fetch a single work order with derived_status computed from its operations.
    /// derived_status is: not_started (0 ops or all pending), in_progress (any started/partial),
    /// complete (all completed).
    pub async fn find_by_id_with_derived(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<WorkOrderResponse>, WorkOrderError> {
        sqlx::query_as::<_, WorkOrderResponse>(
            r#"
            SELECT
                wo.work_order_id,
                wo.tenant_id,
                wo.order_number,
                wo.status,
                wo.item_id,
                wo.bom_revision_id,
                wo.routing_template_id,
                wo.planned_quantity,
                wo.completed_quantity,
                wo.planned_start,
                wo.planned_end,
                wo.actual_start,
                wo.actual_end,
                wo.material_cost_minor,
                wo.labor_cost_minor,
                wo.overhead_cost_minor,
                wo.correlation_id,
                wo.created_at,
                wo.updated_at,
                CASE
                    WHEN COUNT(o.operation_id) = 0 THEN 'not_started'
                    WHEN COUNT(o.operation_id) FILTER (WHERE o.status = 'completed') = COUNT(o.operation_id) THEN 'complete'
                    WHEN COUNT(o.operation_id) FILTER (WHERE o.status IN ('in_progress', 'completed')) > 0 THEN 'in_progress'
                    ELSE 'not_started'
                END AS derived_status
            FROM work_orders wo
            LEFT JOIN operations o
                ON o.work_order_id = wo.work_order_id AND o.tenant_id = wo.tenant_id
            WHERE wo.work_order_id = $1 AND wo.tenant_id = $2
            GROUP BY
                wo.work_order_id, wo.tenant_id, wo.order_number, wo.status, wo.item_id,
                wo.bom_revision_id, wo.routing_template_id, wo.planned_quantity,
                wo.completed_quantity, wo.planned_start, wo.planned_end, wo.actual_start,
                wo.actual_end, wo.material_cost_minor, wo.labor_cost_minor,
                wo.overhead_cost_minor, wo.correlation_id, wo.created_at, wo.updated_at
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(WorkOrderError::Database)
    }

    /// Fetch up to 50 work orders by ID in a single round-trip.
    ///
    /// Uses `= ANY($1)` to avoid N+1 queries.  When `include_operations` or
    /// `include_time_entries` is true a second single-IN query is issued for
    /// each collection and the results are mapped into each WO by ID.
    pub async fn fetch_batch(
        pool: &PgPool,
        ids: &[Uuid],
        tenant_id: &str,
        include_operations: bool,
        include_time_entries: bool,
    ) -> Result<Vec<WorkOrderWithIncludes>, WorkOrderError> {
        let wos = sqlx::query_as::<_, WorkOrderResponse>(
            r#"
            SELECT
                wo.work_order_id, wo.tenant_id, wo.order_number, wo.status,
                wo.item_id, wo.bom_revision_id, wo.routing_template_id,
                wo.planned_quantity, wo.completed_quantity, wo.planned_start,
                wo.planned_end, wo.actual_start, wo.actual_end,
                wo.material_cost_minor, wo.labor_cost_minor, wo.overhead_cost_minor,
                wo.correlation_id, wo.created_at, wo.updated_at,
                CASE
                    WHEN COUNT(o.operation_id) = 0 THEN 'not_started'
                    WHEN COUNT(o.operation_id) FILTER (WHERE o.status = 'completed') = COUNT(o.operation_id) THEN 'complete'
                    WHEN COUNT(o.operation_id) FILTER (WHERE o.status IN ('in_progress', 'completed')) > 0 THEN 'in_progress'
                    ELSE 'not_started'
                END AS derived_status
            FROM work_orders wo
            LEFT JOIN operations o
                ON o.work_order_id = wo.work_order_id AND o.tenant_id = wo.tenant_id
            WHERE wo.work_order_id = ANY($1) AND wo.tenant_id = $2
            GROUP BY
                wo.work_order_id, wo.tenant_id, wo.order_number, wo.status, wo.item_id,
                wo.bom_revision_id, wo.routing_template_id, wo.planned_quantity,
                wo.completed_quantity, wo.planned_start, wo.planned_end, wo.actual_start,
                wo.actual_end, wo.material_cost_minor, wo.labor_cost_minor,
                wo.overhead_cost_minor, wo.correlation_id, wo.created_at, wo.updated_at
            "#,
        )
        .bind(ids)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
        .map_err(WorkOrderError::Database)?;

        let mut result: Vec<WorkOrderWithIncludes> = wos.into_iter().map(Into::into).collect();

        if include_operations && !result.is_empty() {
            let wo_ids: Vec<Uuid> = result.iter().map(|w| w.work_order_id).collect();
            let ops = sqlx::query_as::<_, OperationInstance>(
                "SELECT * FROM operations WHERE work_order_id = ANY($1) AND tenant_id = $2 ORDER BY sequence_number",
            )
            .bind(&wo_ids[..])
            .bind(tenant_id)
            .fetch_all(pool)
            .await
            .map_err(WorkOrderError::Database)?;

            let mut ops_map: HashMap<Uuid, Vec<OperationInstance>> = HashMap::new();
            for op in ops {
                ops_map.entry(op.work_order_id).or_default().push(op);
            }
            for wo in &mut result {
                wo.operations = Some(ops_map.remove(&wo.work_order_id).unwrap_or_default());
            }
        }

        if include_time_entries && !result.is_empty() {
            let wo_ids: Vec<Uuid> = result.iter().map(|w| w.work_order_id).collect();
            let entries = sqlx::query_as::<_, TimeEntry>(
                "SELECT * FROM time_entries WHERE work_order_id = ANY($1) AND tenant_id = $2 ORDER BY start_ts",
            )
            .bind(&wo_ids[..])
            .bind(tenant_id)
            .fetch_all(pool)
            .await
            .map_err(WorkOrderError::Database)?;

            let mut te_map: HashMap<Uuid, Vec<TimeEntry>> = HashMap::new();
            for te in entries {
                te_map.entry(te.work_order_id).or_default().push(te);
            }
            for wo in &mut result {
                wo.time_entries = Some(te_map.remove(&wo.work_order_id).unwrap_or_default());
            }
        }

        Ok(result)
    }

    /// Composite create: allocate a WO number from the Numbering service and
    /// create the work order in a single call.
    ///
    /// `bom_revision_id` and `routing_template_id` are optional.
    /// `idempotency_key` is forwarded to Numbering — re-sending the same key
    /// returns the previously allocated number without creating a duplicate WO.
    pub async fn composite_create(
        pool: &PgPool,
        numbering: &NumberingClient,
        req: &CompositeCreateWorkOrderRequest,
        claims: &VerifiedClaims,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<WorkOrder, WorkOrderError> {
        if req.tenant_id.trim().is_empty() {
            return Err(WorkOrderError::Validation(
                "tenant_id is required".to_string(),
            ));
        }
        if req.planned_quantity <= 0 {
            return Err(WorkOrderError::Validation(
                "planned_quantity must be > 0".to_string(),
            ));
        }

        // Allocate next WO number from the Numbering service.
        let order_number = numbering
            .allocate_wo_number(&req.tenant_id, &req.idempotency_key, claims)
            .await?;

        let mut tx = pool.begin().await?;

        let wo = sqlx::query_as::<_, WorkOrder>(
            r#"
            INSERT INTO work_orders
                (tenant_id, order_number, status, item_id, bom_revision_id,
                 routing_template_id, planned_quantity, planned_start, planned_end)
            VALUES ($1, $2, 'draft', $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(&order_number)
        .bind(req.item_id)
        .bind(req.bom_revision_id)
        .bind(req.routing_template_id)
        .bind(req.planned_quantity)
        .bind(req.planned_start)
        .bind(req.planned_end)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return WorkOrderError::DuplicateOrderNumber(
                        order_number.clone(),
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
                order_number,
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

    /// List work orders for a tenant with derived_status, newest first.
    pub async fn list_with_derived(
        pool: &PgPool,
        tenant_id: &str,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<WorkOrderResponse>, i64), WorkOrderError> {
        let offset = (page - 1) * page_size;

        let rows = sqlx::query_as::<_, WorkOrderResponse>(
            r#"
            SELECT
                wo.work_order_id,
                wo.tenant_id,
                wo.order_number,
                wo.status,
                wo.item_id,
                wo.bom_revision_id,
                wo.routing_template_id,
                wo.planned_quantity,
                wo.completed_quantity,
                wo.planned_start,
                wo.planned_end,
                wo.actual_start,
                wo.actual_end,
                wo.material_cost_minor,
                wo.labor_cost_minor,
                wo.overhead_cost_minor,
                wo.correlation_id,
                wo.created_at,
                wo.updated_at,
                CASE
                    WHEN COUNT(o.operation_id) = 0 THEN 'not_started'
                    WHEN COUNT(o.operation_id) FILTER (WHERE o.status = 'completed') = COUNT(o.operation_id) THEN 'complete'
                    WHEN COUNT(o.operation_id) FILTER (WHERE o.status IN ('in_progress', 'completed')) > 0 THEN 'in_progress'
                    ELSE 'not_started'
                END AS derived_status
            FROM work_orders wo
            LEFT JOIN operations o
                ON o.work_order_id = wo.work_order_id AND o.tenant_id = wo.tenant_id
            WHERE wo.tenant_id = $1
            GROUP BY
                wo.work_order_id, wo.tenant_id, wo.order_number, wo.status, wo.item_id,
                wo.bom_revision_id, wo.routing_template_id, wo.planned_quantity,
                wo.completed_quantity, wo.planned_start, wo.planned_end, wo.actual_start,
                wo.actual_end, wo.material_cost_minor, wo.labor_cost_minor,
                wo.overhead_cost_minor, wo.correlation_id, wo.created_at, wo.updated_at
            ORDER BY wo.created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(tenant_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(WorkOrderError::Database)?;

        let total: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM work_orders WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(pool)
                .await
                .map_err(WorkOrderError::Database)?;

        Ok((rows, total.0))
    }
}
