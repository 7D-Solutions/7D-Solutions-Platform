//! `From<DomainError> for ApiError` conversions.
//!
//! Centralised mapping so every handler can use `ApiError` directly instead of
//! inline `json!` error construction.  Status codes and error codes match the
//! pre-migration handler behaviour exactly — no semantic changes.

use platform_http_contracts::ApiError;

use super::adjust_service::AdjustError;
use super::classifications::ClassificationError;
use super::cycle_count::approve_service::ApproveError;
use super::cycle_count::submit_service::SubmitError;
use super::cycle_count::task_service::TaskError;
use super::expiry::ExpiryError;
use super::fifo::FifoError;
use super::fulfill_service::FulfillError;
use super::genealogy::GenealogyError;
use super::guards::GuardError;
use super::history::change_history::ChangeHistoryError;
use super::issue_service::IssueError;
use super::items::ItemError;
use super::labels::LabelError;
use super::locations::LocationError;
use super::lots_serials::issue::LotSerialError;
use super::make_buy::MakeBuyError;
use super::receipt_service::ReceiptError;
use super::reorder::models::ReorderPolicyError;
use super::reservation_service::ReservationError;
use super::revisions::RevisionError;
use super::status::transfer_service::StatusTransferError;
use super::transfer_service::TransferError;
use super::uom::convert::ConvertError;
use super::uom::models::UomError;
use super::valuation::run_service::RunError;
use super::valuation::snapshot_service::SnapshotError;

// ── GuardError ────────────────────────────────────────────────────────────

impl From<GuardError> for ApiError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::ItemNotFound => {
                ApiError::not_found("Item not found or does not belong to this tenant")
            }
            GuardError::ItemInactive => ApiError::new(422, "item_inactive", err.to_string()),
            GuardError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            GuardError::NoBaseUom => ApiError::new(422, "no_base_uom", err.to_string()),
            GuardError::UomConversion(e) => {
                ApiError::new(422, "uom_conversion_error", e.to_string())
            }
            GuardError::Database(e) => {
                tracing::error!(error = %e, "guard database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── ItemError ─────────────────────────────────────────────────────────────

impl From<ItemError> for ApiError {
    fn from(err: ItemError) -> Self {
        match err {
            ItemError::DuplicateSku(sku, tenant) => ApiError::conflict(format!(
                "SKU '{}' already exists for tenant '{}'",
                sku, tenant
            )),
            ItemError::NotFound => ApiError::not_found("Item not found"),
            ItemError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            ItemError::Database(e) => {
                tracing::error!(error = %e, "item database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── LocationError ─────────────────────────────────────────────────────────

impl From<LocationError> for ApiError {
    fn from(err: LocationError) -> Self {
        match err {
            LocationError::DuplicateCode(code, wid, tenant) => ApiError::conflict(format!(
                "Location code '{}' already exists for warehouse '{}' in tenant '{}'",
                code, wid, tenant
            )),
            LocationError::NotFound => ApiError::not_found("Location not found"),
            LocationError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            LocationError::Database(e) => {
                tracing::error!(error = %e, "location database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── LabelError ────────────────────────────────────────────────────────────

impl From<LabelError> for ApiError {
    fn from(err: LabelError) -> Self {
        match err {
            LabelError::ItemNotFound | LabelError::RevisionNotFound => {
                ApiError::not_found(err.to_string())
            }
            LabelError::ItemInactive => ApiError::conflict(err.to_string()),
            LabelError::RevisionMismatch => {
                ApiError::new(422, "revision_mismatch", err.to_string())
            }
            LabelError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            LabelError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            LabelError::Serialization(e) => {
                tracing::error!(error = %e, "label serialization error");
                ApiError::internal("Serialization error")
            }
            LabelError::Database(e) => {
                tracing::error!(error = %e, "label database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── RevisionError ─────────────────────────────────────────────────────────

impl From<RevisionError> for ApiError {
    fn from(err: RevisionError) -> Self {
        match err {
            RevisionError::ItemNotFound | RevisionError::RevisionNotFound => {
                ApiError::not_found(err.to_string())
            }
            RevisionError::ItemInactive => ApiError::conflict(err.to_string()),
            RevisionError::AlreadyActivated => ApiError::conflict(err.to_string()),
            RevisionError::PolicyLockedOnActivatedRevision => ApiError::conflict(err.to_string()),
            RevisionError::OverlappingWindow => ApiError::conflict(err.to_string()),
            RevisionError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            RevisionError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            RevisionError::Serialization(e) => {
                tracing::error!(error = %e, "revision serialization error");
                ApiError::internal("Serialization error")
            }
            RevisionError::Database(e) => {
                tracing::error!(error = %e, "revision database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── ReorderPolicyError ────────────────────────────────────────────────────

impl From<ReorderPolicyError> for ApiError {
    fn from(err: ReorderPolicyError) -> Self {
        match err {
            ReorderPolicyError::NotFound => ApiError::not_found("Reorder policy not found"),
            ReorderPolicyError::DuplicatePolicy => ApiError::conflict(err.to_string()),
            ReorderPolicyError::ItemNotFound => {
                ApiError::not_found("Item not found or does not belong to this tenant")
            }
            ReorderPolicyError::LocationNotFound => ApiError::not_found(
                "Location not found, inactive, or does not belong to this tenant",
            ),
            ReorderPolicyError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            ReorderPolicyError::Database(e) => {
                tracing::error!(error = %e, "database error in reorder policy handler");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── SnapshotError ─────────────────────────────────────────────────────────

impl From<SnapshotError> for ApiError {
    fn from(err: SnapshotError) -> Self {
        match err {
            SnapshotError::MissingTenant | SnapshotError::MissingIdempotencyKey => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            SnapshotError::ConcurrentSnapshot => ApiError::conflict(err.to_string()),
            SnapshotError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            SnapshotError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in valuation snapshot");
                ApiError::internal("Serialization error")
            }
            SnapshotError::Database(e) => {
                tracing::error!(error = %e, "database error creating valuation snapshot");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── RunError (valuation) ──────────────────────────────────────────────────

impl From<RunError> for ApiError {
    fn from(err: RunError) -> Self {
        match err {
            RunError::MissingTenant | RunError::MissingIdempotencyKey => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            RunError::ConcurrentRun => ApiError::conflict(err.to_string()),
            RunError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            RunError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in valuation run");
                ApiError::internal("Serialization error")
            }
            RunError::Database(e) => {
                tracing::error!(error = %e, "database error in valuation run");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── UomError ──────────────────────────────────────────────────────────────

impl From<UomError> for ApiError {
    fn from(err: UomError) -> Self {
        match err {
            UomError::DuplicateCode(code, tenant) => ApiError::conflict(format!(
                "UoM code '{}' already exists for tenant '{}'",
                code, tenant
            )),
            UomError::DuplicateConversion => ApiError::conflict(err.to_string()),
            UomError::NotFound => ApiError::not_found("UoM not found"),
            UomError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            UomError::Database(e) => {
                tracing::error!(error = %e, "uom database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── ConvertError ──────────────────────────────────────────────────────────

impl From<ConvertError> for ApiError {
    fn from(err: ConvertError) -> Self {
        ApiError::new(422, "uom_conversion_error", err.to_string())
    }
}

// ── FifoError ─────────────────────────────────────────────────────────────

impl From<FifoError> for ApiError {
    fn from(err: FifoError) -> Self {
        ApiError::new(422, "fifo_error", err.to_string())
    }
}

// ── AdjustError ───────────────────────────────────────────────────────────

impl From<AdjustError> for ApiError {
    fn from(err: AdjustError) -> Self {
        match err {
            AdjustError::Guard(ge) => ge.into(),
            AdjustError::NegativeOnHand {
                available,
                would_be,
            } => ApiError::new(
                422,
                "negative_on_hand",
                format!(
                    "Adjustment would drive on-hand negative: have {}, would become {}",
                    available, would_be
                ),
            ),
            AdjustError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            AdjustError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in adjustment");
                ApiError::internal("Serialization error")
            }
            AdjustError::Database(e) => {
                tracing::error!(error = %e, "database error in adjustment");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── ReceiptError ──────────────────────────────────────────────────────────

impl From<ReceiptError> for ApiError {
    fn from(err: ReceiptError) -> Self {
        match err {
            ReceiptError::Guard(ge) => ge.into(),
            ReceiptError::LotCodeRequired
            | ReceiptError::SerialCodesRequired
            | ReceiptError::ExpiryPolicy(_) => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            ReceiptError::SerialCountMismatch { .. } => {
                ApiError::new(422, "serial_count_mismatch", err.to_string())
            }
            ReceiptError::DuplicateSerialCode => ApiError::conflict(err.to_string()),
            ReceiptError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            ReceiptError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in receipt");
                ApiError::internal("Serialization error")
            }
            ReceiptError::Database(e) => {
                tracing::error!(error = %e, "database error in receipt");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── IssueError ────────────────────────────────────────────────────────────

impl From<IssueError> for ApiError {
    fn from(err: IssueError) -> Self {
        match err {
            IssueError::Guard(ge) => ge.into(),
            IssueError::InsufficientQuantity { .. } => {
                ApiError::new(422, "insufficient_stock", err.to_string())
            }
            IssueError::Fifo(fe) => fe.into(),
            IssueError::NoLayersAvailable => {
                ApiError::new(422, "no_layers_available", err.to_string())
            }
            IssueError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            IssueError::LotRequired | IssueError::SerialRequired => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            IssueError::LotNotFound(_) => ApiError::not_found(err.to_string()),
            IssueError::SerialNotAvailable(_) => {
                ApiError::new(422, "serial_not_available", err.to_string())
            }
            IssueError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in issue");
                ApiError::internal("Serialization error")
            }
            IssueError::Database(e) => {
                tracing::error!(error = %e, "database error in issue");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── TransferError ─────────────────────────────────────────────────────────

impl From<TransferError> for ApiError {
    fn from(err: TransferError) -> Self {
        match err {
            TransferError::Guard(ge) => ge.into(),
            TransferError::SameWarehouse => ApiError::new(422, "validation_error", err.to_string()),
            TransferError::InsufficientQuantity { .. } => {
                ApiError::new(422, "insufficient_stock", err.to_string())
            }
            TransferError::Fifo(fe) => fe.into(),
            TransferError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            TransferError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in transfer");
                ApiError::internal("Serialization error")
            }
            TransferError::Database(e) => {
                tracing::error!(error = %e, "database error in transfer");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── ReservationError ──────────────────────────────────────────────────────

impl From<ReservationError> for ApiError {
    fn from(err: ReservationError) -> Self {
        match err {
            ReservationError::Guard(ge) => ge.into(),
            ReservationError::ReservationNotFound => ApiError::not_found("Reservation not found"),
            ReservationError::AlreadyReleased => ApiError::conflict(err.to_string()),
            ReservationError::InsufficientAvailable { .. } => {
                ApiError::new(422, "insufficient_stock", err.to_string())
            }
            ReservationError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            ReservationError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in reservation");
                ApiError::internal("Serialization error")
            }
            ReservationError::Database(e) => {
                tracing::error!(error = %e, "database error in reservation");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── FulfillError ──────────────────────────────────────────────────────────

impl From<FulfillError> for ApiError {
    fn from(err: FulfillError) -> Self {
        match err {
            FulfillError::Guard(ge) => ge.into(),
            FulfillError::ReservationNotFound => ApiError::not_found("Reservation not found"),
            FulfillError::AlreadySettled => ApiError::conflict(err.to_string()),
            FulfillError::QuantityExceedsReserved(_, _) => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            FulfillError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            FulfillError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in fulfillment");
                ApiError::internal("Serialization error")
            }
            FulfillError::Database(e) => {
                tracing::error!(error = %e, "database error in fulfillment");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── ExpiryError ───────────────────────────────────────────────────────────

impl From<ExpiryError> for ApiError {
    fn from(err: ExpiryError) -> Self {
        match err {
            ExpiryError::LotNotFound => ApiError::not_found("Lot not found"),
            ExpiryError::ExpiryDateRequired | ExpiryError::MissingShelfLifePolicy => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            ExpiryError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            ExpiryError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            ExpiryError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in expiry");
                ApiError::internal("Serialization error")
            }
            ExpiryError::Database(e) => {
                tracing::error!(error = %e, "database error in expiry");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── GenealogyError ────────────────────────────────────────────────────────

impl From<GenealogyError> for ApiError {
    fn from(err: GenealogyError) -> Self {
        match err {
            GenealogyError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            GenealogyError::LotNotFound(_) => ApiError::not_found(err.to_string()),
            GenealogyError::QuantityConservation { .. } => {
                ApiError::new(422, "quantity_conservation", err.to_string())
            }
            GenealogyError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            GenealogyError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in genealogy");
                ApiError::internal("Serialization error")
            }
            GenealogyError::Database(e) => {
                tracing::error!(error = %e, "database error in genealogy");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── ClassificationError ───────────────────────────────────────────────────

impl From<ClassificationError> for ApiError {
    fn from(err: ClassificationError) -> Self {
        match err {
            ClassificationError::ItemNotFound => ApiError::not_found("Item not found"),
            ClassificationError::ItemInactive => ApiError::conflict(err.to_string()),
            ClassificationError::DuplicateAssignment => ApiError::conflict(err.to_string()),
            ClassificationError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            ClassificationError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            ClassificationError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in classification");
                ApiError::internal("Serialization error")
            }
            ClassificationError::Database(e) => {
                tracing::error!(error = %e, "database error in classification");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── ChangeHistoryError ────────────────────────────────────────────────────

impl From<ChangeHistoryError> for ApiError {
    fn from(err: ChangeHistoryError) -> Self {
        match err {
            ChangeHistoryError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            ChangeHistoryError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            ChangeHistoryError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in change history");
                ApiError::internal("Serialization error")
            }
            ChangeHistoryError::Database(e) => {
                tracing::error!(error = %e, "database error in change history");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── MakeBuyError ──────────────────────────────────────────────────────────

impl From<MakeBuyError> for ApiError {
    fn from(err: MakeBuyError) -> Self {
        match err {
            MakeBuyError::NotFound => ApiError::not_found("Item not found"),
            MakeBuyError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            MakeBuyError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in make/buy");
                ApiError::internal("Serialization error")
            }
            MakeBuyError::Database(e) => {
                tracing::error!(error = %e, "database error in make/buy");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── LotSerialError ────────────────────────────────────────────────────────

impl From<LotSerialError> for ApiError {
    fn from(err: LotSerialError) -> Self {
        match err {
            LotSerialError::SerialNotAvailable(code) => ApiError::new(
                422,
                "serial_not_available",
                format!("Serial '{}' is not available", code),
            ),
            LotSerialError::Database(e) => {
                tracing::error!(error = %e, "database error in lot/serial");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── StatusTransferError ───────────────────────────────────────────────────

impl From<StatusTransferError> for ApiError {
    fn from(err: StatusTransferError) -> Self {
        match err {
            StatusTransferError::Guard(ge) => ge.into(),
            StatusTransferError::SameStatus => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            StatusTransferError::InsufficientStock { .. } => {
                ApiError::new(422, "insufficient_stock", err.to_string())
            }
            StatusTransferError::BucketNotFound(_) => ApiError::not_found(err.to_string()),
            StatusTransferError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            StatusTransferError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in status transfer");
                ApiError::internal("Serialization error")
            }
            StatusTransferError::Database(e) => {
                tracing::error!(error = %e, "database error in status transfer");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── TaskError (cycle count) ───────────────────────────────────────────────

impl From<TaskError> for ApiError {
    fn from(err: TaskError) -> Self {
        match err {
            TaskError::MissingTenant | TaskError::EmptyPartialItemList => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            TaskError::LocationNotFound => ApiError::not_found(err.to_string()),
            TaskError::Database(e) => {
                tracing::error!(error = %e, "database error in cycle count task");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── SubmitError (cycle count) ─────────────────────────────────────────────

impl From<SubmitError> for ApiError {
    fn from(err: SubmitError) -> Self {
        match err {
            SubmitError::MissingTenant | SubmitError::MissingIdempotencyKey => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            SubmitError::TaskNotFound => ApiError::not_found(err.to_string()),
            SubmitError::TaskNotOpen { .. } => ApiError::conflict(err.to_string()),
            SubmitError::LineNotFound { .. } | SubmitError::NegativeCountedQty { .. } => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            SubmitError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            SubmitError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in cycle count submit");
                ApiError::internal("Serialization error")
            }
            SubmitError::Database(e) => {
                tracing::error!(error = %e, "database error in cycle count submit");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── ApproveError (cycle count) ────────────────────────────────────────────

impl From<ApproveError> for ApiError {
    fn from(err: ApproveError) -> Self {
        match err {
            ApproveError::MissingTenant | ApproveError::MissingIdempotencyKey => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            ApproveError::TaskNotFound => ApiError::not_found(err.to_string()),
            ApproveError::TaskNotSubmitted { .. } => ApiError::conflict(err.to_string()),
            ApproveError::ConflictingIdempotencyKey => ApiError::conflict(err.to_string()),
            ApproveError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error in cycle count approve");
                ApiError::internal("Serialization error")
            }
            ApproveError::Database(e) => {
                tracing::error!(error = %e, "database error in cycle count approve");
                ApiError::internal("Database error")
            }
        }
    }
}
