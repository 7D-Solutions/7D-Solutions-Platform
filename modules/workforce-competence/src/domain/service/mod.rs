//! Workforce competence service.
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)
//!
//! Invariants:
//! - All mutations are tenant-scoped
//! - Idempotency key prevents double-processing on retry
//! - Authorization queries are time-aware and never delete history

pub mod core;
pub mod queries;

pub use core::{
    assign_competence, register_artifact, ArtifactRegisteredPayload, CompetenceAssignedPayload,
    ServiceError,
};
pub use queries::{check_authorization, get_artifact};
