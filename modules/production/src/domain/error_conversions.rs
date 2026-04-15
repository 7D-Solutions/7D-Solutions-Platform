//! `From<DomainError> for ApiError` conversions.
//!
//! Centralised mapping so every handler can use `ApiError` directly instead of
//! inline `json!` error construction.

use platform_http_contracts::ApiError;

use super::component_issue::ComponentIssueError;
use super::downtime::DowntimeError;
use super::fg_receipt::FgReceiptError;
use super::operations::OperationError;
use super::routings::RoutingError;
use super::time_entries::TimeEntryError;
use super::work_orders::WorkOrderError;
use super::workcenters::WorkcenterError;

// ── WorkcenterError ──────────────────────────────────────────────────────

impl From<WorkcenterError> for ApiError {
    fn from(err: WorkcenterError) -> Self {
        match err {
            WorkcenterError::DuplicateCode(code, tenant) => ApiError::conflict(format!(
                "Workcenter code '{}' already exists for tenant '{}'",
                code, tenant
            )),
            WorkcenterError::NotFound => ApiError::not_found("Workcenter not found"),
            WorkcenterError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            WorkcenterError::ConflictingIdempotencyKey => {
                ApiError::new(409, "conflict", "Conflicting idempotency key")
            }
            WorkcenterError::Database(e) => {
                tracing::error!(error = %e, "workcenter database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── WorkOrderError ───────────────────────────────────────────────────────

impl From<WorkOrderError> for ApiError {
    fn from(err: WorkOrderError) -> Self {
        match err {
            WorkOrderError::DuplicateOrderNumber(num, tenant) => ApiError::conflict(format!(
                "Order number '{}' already exists for tenant '{}'",
                num, tenant
            )),
            WorkOrderError::DuplicateCorrelation => {
                ApiError::conflict("Work order with this correlation_id already exists")
            }
            WorkOrderError::NotFound => ApiError::not_found("Work order not found"),
            WorkOrderError::InvalidTransition { from, to } => ApiError::new(
                422,
                "invalid_transition",
                format!("Cannot transition from '{}' to '{}'", from, to),
            ),
            WorkOrderError::BomRevisionSuperseded {
                revision_id,
                eco_number,
                new_rev_id,
            } => ApiError::new(
                422,
                "BOM_REVISION_SUPERSEDED",
                format!(
                    "BOM revision {} was superseded by ECO {}. Use revision {} instead.",
                    revision_id, eco_number, new_rev_id
                ),
            ),
            WorkOrderError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            WorkOrderError::NumberingService(msg) => {
                tracing::error!(error = %msg, "numbering service error");
                ApiError::new(503, "numbering_service_error", msg)
            }
            WorkOrderError::Database(e) => {
                tracing::error!(error = %e, "work order database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── OperationError ───────────────────────────────────────────────────────

impl From<OperationError> for ApiError {
    fn from(err: OperationError) -> Self {
        match err {
            OperationError::NotFound => ApiError::not_found("Operation not found"),
            OperationError::WorkOrderNotFound => ApiError::not_found("Work order not found"),
            OperationError::WorkOrderNotReleased => ApiError::new(
                422,
                "work_order_not_released",
                "Work order must be in 'released' status",
            ),
            OperationError::NoRoutingTemplate => ApiError::new(
                422,
                "no_routing_template",
                "Work order has no routing template assigned",
            ),
            OperationError::AlreadyInitialized => {
                ApiError::conflict("Operations already initialized for this work order")
            }
            OperationError::InvalidTransition { from, to } => ApiError::new(
                422,
                "invalid_transition",
                format!("Cannot transition from '{}' to '{}'", from, to),
            ),
            OperationError::PredecessorNotComplete(seq) => ApiError::new(
                422,
                "predecessor_not_complete",
                format!("Predecessor operation (seq {}) is not completed", seq),
            ),
            OperationError::Database(e) => {
                tracing::error!(error = %e, "operation database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── TimeEntryError ───────────────────────────────────────────────────────

impl From<TimeEntryError> for ApiError {
    fn from(err: TimeEntryError) -> Self {
        match err {
            TimeEntryError::NotFound => ApiError::not_found("Time entry not found"),
            TimeEntryError::WorkOrderNotFound => ApiError::not_found("Work order not found"),
            TimeEntryError::OperationNotFound => ApiError::new(
                422,
                "operation_not_found",
                "Operation not found or does not belong to work order",
            ),
            TimeEntryError::AlreadyStopped => ApiError::conflict("Timer has already been stopped"),
            TimeEntryError::InvalidTimeRange => ApiError::new(
                422,
                "invalid_time_range",
                "End time must be after start time",
            ),
            TimeEntryError::ConflictingIdempotencyKey => {
                ApiError::new(409, "conflict", "Conflicting idempotency key")
            }
            TimeEntryError::Database(e) => {
                tracing::error!(error = %e, "time entry database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── DowntimeError ────────────────────────────────────────────────────────

impl From<DowntimeError> for ApiError {
    fn from(err: DowntimeError) -> Self {
        match err {
            DowntimeError::NotFound => ApiError::not_found("Downtime record not found"),
            DowntimeError::WorkcenterNotFound => ApiError::not_found("Workcenter not found"),
            DowntimeError::AlreadyEnded => ApiError::conflict("Downtime already ended"),
            DowntimeError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            DowntimeError::ConflictingIdempotencyKey => {
                ApiError::new(409, "conflict", "Conflicting idempotency key")
            }
            DowntimeError::Database(e) => {
                tracing::error!(error = %e, "downtime database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── RoutingError ─────────────────────────────────────────────────────────

impl From<RoutingError> for ApiError {
    fn from(err: RoutingError) -> Self {
        match err {
            RoutingError::DuplicateRevision(rev, tenant) => ApiError::conflict(format!(
                "Routing revision '{}' already exists for item in tenant '{}'",
                rev, tenant
            )),
            RoutingError::DuplicateSequence(seq) => ApiError::conflict(format!(
                "Sequence number {} already exists for this routing",
                seq
            )),
            RoutingError::WorkcenterInvalid(id) => ApiError::new(
                422,
                "workcenter_invalid",
                format!("Workcenter '{}' not found or inactive", id),
            ),
            RoutingError::NotFound => ApiError::not_found("Routing not found"),
            RoutingError::StepNotFound => ApiError::not_found("Routing step not found"),
            RoutingError::InvalidTransition { from, to } => {
                ApiError::conflict(format!("Cannot transition from '{}' to '{}'", from, to))
            }
            RoutingError::ReleasedImmutable => {
                ApiError::conflict("Cannot modify a released routing")
            }
            RoutingError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            RoutingError::ConflictingIdempotencyKey => {
                ApiError::new(409, "conflict", "Conflicting idempotency key")
            }
            RoutingError::Database(e) => {
                tracing::error!(error = %e, "routing database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── ComponentIssueError ──────────────────────────────────────────────────

impl From<ComponentIssueError> for ApiError {
    fn from(err: ComponentIssueError) -> Self {
        match err {
            ComponentIssueError::NotFound => ApiError::not_found("Work order not found"),
            ComponentIssueError::NotReleased => ApiError::new(
                422,
                "not_released",
                "Work order must be in 'released' status",
            ),
            ComponentIssueError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            ComponentIssueError::ConflictingIdempotencyKey => {
                ApiError::new(409, "conflict", "Conflicting idempotency key")
            }
            ComponentIssueError::Database(e) => {
                tracing::error!(error = %e, "component issue database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── FgReceiptError ───────────────────────────────────────────────────────

impl From<FgReceiptError> for ApiError {
    fn from(err: FgReceiptError) -> Self {
        match err {
            FgReceiptError::NotFound => ApiError::not_found("Work order not found"),
            FgReceiptError::NotReleased => ApiError::new(
                422,
                "not_released",
                "Work order must be in 'released' status",
            ),
            FgReceiptError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            FgReceiptError::ConflictingIdempotencyKey => {
                ApiError::new(409, "conflict", "Conflicting idempotency key")
            }
            FgReceiptError::Database(e) => {
                tracing::error!(error = %e, "fg receipt database error");
                ApiError::internal("Database error")
            }
        }
    }
}
