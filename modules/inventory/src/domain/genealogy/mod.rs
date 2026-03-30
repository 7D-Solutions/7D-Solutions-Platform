//! Lot genealogy service: split and merge operations.
//!
//! Records immutable parent-child edges between lots representing material
//! transformations. Each operation creates one or more edges grouped by a
//! shared `operation_id`.
//!
//! Invariants:
//! - Guard → Mutation → Outbox atomicity (single transaction)
//! - Idempotent via `idempotency_key` (tenant-scoped)
//! - No cross-tenant edges
//! - No self-referencing edges (parent != child)
//! - All lots must belong to the same item and tenant

mod helpers;
pub mod merge;
pub mod queries;
pub mod split;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Re-exports
// ============================================================================

pub use merge::process_merge;
pub use queries::{children_of, parents_of, GenealogyEdge};
pub use split::process_split;

// ============================================================================
// Types
// ============================================================================

/// Input for a lot split operation.
///
/// A split takes one parent lot and distributes quantity to one or more
/// child lots. Child lots are created if they don't already exist.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LotSplitRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub parent_lot_code: String,
    pub children: Vec<SplitChild>,
    pub actor_id: Option<Uuid>,
    pub notes: Option<String>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SplitChild {
    pub lot_code: String,
    pub quantity: i64,
}

/// Input for a lot merge operation.
///
/// A merge takes multiple parent lots and combines their quantity into a
/// single child lot. The child lot is created if it doesn't already exist.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LotMergeRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub parents: Vec<MergeParent>,
    pub child_lot_code: String,
    pub actor_id: Option<Uuid>,
    pub notes: Option<String>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MergeParent {
    pub lot_code: String,
    pub quantity: i64,
}

/// Result returned on successful split or merge.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GenealogyResult {
    pub operation_id: Uuid,
    pub edge_count: usize,
    pub event_id: Uuid,
}

#[derive(Debug, Error)]
pub enum GenealogyError {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Lot not found: {0}")]
    LotNotFound(String),

    #[error("Quantity conservation violated: children sum to {children_sum} but parent has {parent_qty} on hand")]
    QuantityConservation { children_sum: i64, parent_qty: i64 },

    #[error("Idempotency key conflict: same key used with a different request body")]
    ConflictingIdempotencyKey,

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
