//! Item revision management with effective dating.
//!
//! Revisions are the audit spine of item master data. Each revision captures
//! a snapshot of the item definition (name, UoM, GL accounts) and can be
//! activated for an effective window [effective_from, effective_to).
//!
//! Invariants:
//! - revision_number auto-increments per (tenant_id, item_id)
//! - Effective windows are non-overlapping (DB exclusion constraint)
//! - Activating a new revision auto-closes any open-ended predecessor
//! - Idempotent creation and activation via idempotency_key
//! - All writes follow Guard → Mutation → Outbox pattern

mod models;
mod queries;
mod service;

// Re-export public API to preserve existing import paths.
pub use models::{
    ActivateRevisionRequest, CreateRevisionRequest, ItemRevision, RevisionError,
    UpdateRevisionPolicyRequest,
};
pub use queries::{list_revisions, revision_at};
pub use service::{activate_revision, create_revision, update_revision_policy};
