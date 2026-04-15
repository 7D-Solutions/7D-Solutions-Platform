//! `From<DomainError> for ApiError` conversions.
//!
//! Centralised mapping so every handler can use `ApiError` directly instead of
//! inline error construction.  Status codes and error codes match the
//! pre-migration handler behaviour exactly — no semantic changes.

use platform_http_contracts::ApiError;

use super::assets::AssetError;
use super::calibration::CalibrationError;
use super::calibration_events::CalibrationEventError;
use super::downtime::DowntimeError;
use super::meters::MeterError;
use super::plans::PlanError;
use super::work_orders::{WoError, WoLaborError, WoPartError};

// ── AssetError ───────────────────────────────────────────────────────────

impl From<AssetError> for ApiError {
    fn from(err: AssetError) -> Self {
        match err {
            AssetError::DuplicateTag(tag, tenant) => ApiError::conflict(format!(
                "Asset tag '{}' already exists for tenant '{}'",
                tag, tenant
            )),
            AssetError::NotFound => ApiError::not_found("Asset not found"),
            AssetError::Validation(msg) => ApiError::bad_request(msg),
            AssetError::IdempotentDuplicate(_) => {
                ApiError::new(200, "idempotent_duplicate", "Asset already exists")
            }
            AssetError::Database(e) => {
                tracing::error!(error = %e, "asset database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── WoError ──────────────────────────────────────────────────────────────

impl From<WoError> for ApiError {
    fn from(err: WoError) -> Self {
        match err {
            WoError::NotFound => ApiError::not_found("Work order not found"),
            WoError::AssetNotFound => ApiError::not_found("Asset not found"),
            WoError::AssignmentNotFound => ApiError::not_found("Plan assignment not found"),
            WoError::Validation(msg) => ApiError::bad_request(msg),
            WoError::Transition(e) => ApiError::new(422, "invalid_transition", e.to_string()),
            WoError::Guard(e) => ApiError::new(422, "guard_failed", e.to_string()),
            WoError::Database(e) => {
                tracing::error!(error = %e, "work order database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── WoPartError ──────────────────────────────────────────────────────────

impl From<WoPartError> for ApiError {
    fn from(err: WoPartError) -> Self {
        match err {
            WoPartError::WoNotFound => ApiError::not_found("Work order not found"),
            WoPartError::PartNotFound => ApiError::not_found("Part not found"),
            WoPartError::WoImmutable(status) => ApiError::new(
                422,
                "wo_immutable",
                format!("Cannot modify parts: work order status is {}", status),
            ),
            WoPartError::Validation(msg) => ApiError::bad_request(msg),
            WoPartError::Database(e) => {
                tracing::error!(error = %e, "work order parts database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── WoLaborError ─────────────────────────────────────────────────────────

impl From<WoLaborError> for ApiError {
    fn from(err: WoLaborError) -> Self {
        match err {
            WoLaborError::WoNotFound => ApiError::not_found("Work order not found"),
            WoLaborError::LaborNotFound => ApiError::not_found("Labor entry not found"),
            WoLaborError::WoImmutable(status) => ApiError::new(
                422,
                "wo_immutable",
                format!("Cannot modify labor: work order status is {}", status),
            ),
            WoLaborError::Validation(msg) => ApiError::bad_request(msg),
            WoLaborError::Database(e) => {
                tracing::error!(error = %e, "work order labor database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── PlanError ────────────────────────────────────────────────────────────

impl From<PlanError> for ApiError {
    fn from(err: PlanError) -> Self {
        match err {
            PlanError::PlanNotFound => ApiError::not_found("Plan not found"),
            PlanError::AssetNotFound => ApiError::not_found("Asset not found"),
            PlanError::MeterTypeNotFound => ApiError::not_found("Meter type not found"),
            PlanError::DuplicateAssignment => {
                ApiError::conflict("This plan is already assigned to this asset")
            }
            PlanError::AssignmentNotFound => ApiError::not_found("Assignment not found"),
            PlanError::Validation(msg) => ApiError::bad_request(msg),
            PlanError::Database(e) => {
                tracing::error!(error = %e, "plan database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── MeterError ───────────────────────────────────────────────────────────

impl From<MeterError> for ApiError {
    fn from(err: MeterError) -> Self {
        match err {
            MeterError::DuplicateName(name, tenant) => ApiError::conflict(format!(
                "Meter type '{}' already exists for tenant '{}'",
                name, tenant
            )),
            MeterError::MeterTypeNotFound => ApiError::not_found("Meter type not found"),
            MeterError::AssetNotFound => ApiError::not_found("Asset not found"),
            MeterError::MonotonicityViolation {
                previous,
                attempted,
            } => ApiError::bad_request(format!(
                "Reading {} violates monotonicity: previous max was {}",
                attempted, previous
            )),
            MeterError::Validation(msg) => ApiError::bad_request(msg),
            MeterError::Database(e) => {
                tracing::error!(error = %e, "meter database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── DowntimeError ────────────────────────────────────────────────────────

impl From<DowntimeError> for ApiError {
    fn from(err: DowntimeError) -> Self {
        match err {
            DowntimeError::NotFound => ApiError::not_found("Downtime event not found"),
            DowntimeError::AssetNotFound => ApiError::not_found("Asset not found"),
            DowntimeError::Validation(msg) => ApiError::bad_request(msg),
            DowntimeError::IdempotentDuplicate(event) => ApiError::new(
                200,
                "idempotent_duplicate",
                format!("Downtime event {} already exists", event.id),
            ),
            DowntimeError::Database(e) => {
                tracing::error!(error = %e, "downtime database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── CalibrationEventError ────────────────────────────────────────────────

impl From<CalibrationEventError> for ApiError {
    fn from(err: CalibrationEventError) -> Self {
        match err {
            CalibrationEventError::AssetNotFound => ApiError::not_found("Asset not found"),
            CalibrationEventError::Validation(msg) => ApiError::bad_request(msg),
            CalibrationEventError::IdempotentDuplicate(_) => ApiError::new(
                200,
                "idempotent_duplicate",
                "Calibration event already exists",
            ),
            CalibrationEventError::Database(e) => {
                tracing::error!(error = %e, "calibration event database error");
                ApiError::internal("Database error")
            }
        }
    }
}

// ── CalibrationError ─────────────────────────────────────────────────────

impl From<CalibrationError> for ApiError {
    fn from(err: CalibrationError) -> Self {
        match err {
            CalibrationError::NotFound => ApiError::not_found("Calibration record not found"),
            CalibrationError::AssetNotFound => ApiError::not_found("Asset not found"),
            CalibrationError::Validation(msg) => ApiError::bad_request(msg),
            CalibrationError::AlreadyCompleted => {
                ApiError::conflict("Calibration already completed — immutable")
            }
            CalibrationError::DuplicateKey => ApiError::conflict("Duplicate idempotency key"),
            CalibrationError::Database(e) => {
                tracing::error!(error = %e, "calibration database error");
                ApiError::internal("Database error")
            }
        }
    }
}
