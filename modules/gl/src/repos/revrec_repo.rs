//! Revrec repository — re-exports from contract, schedule, and amendment sub-modules
//!
//! Split by concern:
//! - `revrec_contract_repo` — contract + obligation CRUD
//! - `revrec_schedule_repo` — schedule CRUD + recognition run support
//! - `revrec_amendment_repo` — contract amendments + schedule supersession

use uuid::Uuid;

// Re-export everything for backward compatibility
pub use super::revrec_amendment_repo::*;
pub use super::revrec_contract_repo::*;
pub use super::revrec_schedule_repo::*;

/// Errors from revrec repository operations
#[derive(Debug, thiserror::Error)]
pub enum RevrecRepoError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Contract already exists: {0}")]
    DuplicateContract(Uuid),

    #[error("Allocation sum mismatch: obligations sum to {sum}, expected {expected}")]
    AllocationMismatch { sum: i64, expected: i64 },

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Schedule already exists: {0}")]
    DuplicateSchedule(Uuid),

    #[error("Obligation not found: {0}")]
    ObligationNotFound(Uuid),

    #[error("Schedule lines sum {sum} does not match total {expected}")]
    ScheduleSumMismatch { sum: i64, expected: i64 },

    #[error("Modification already exists: {0}")]
    DuplicateModification(Uuid),

    #[error("Contract not found: {0}")]
    ContractNotFound(Uuid),
}
