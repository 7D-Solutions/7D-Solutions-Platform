//! Platform-level projection infrastructure
//!
//! This crate provides the foundational types and contracts for projection management:
//! - Cursor tracking for event stream position (bd-17a0)
//! - Idempotent event apply semantics (bd-17a0)
//! - Metrics for projection health monitoring (future beads)
//!
//! # Cursor Tracking
//!
//! The cursor module provides a projection contract that enforces:
//! 1. Per-projection cursor persistence (keyed by projection_name + tenant_id)
//! 2. Idempotent apply semantics (events are never applied twice)
//! 3. Transactional cursor updates with read-model writes
//! 4. Deterministic rebuild capability
//!
//! # Example
//!
//! ```rust,no_run
//! use projections::cursor::{try_apply_event, ProjectionCursor};
//! use uuid::Uuid;
//! use chrono::Utc;
//!
//! async fn process_event(
//!     tx: &mut sqlx::PgConnection,
//!     event_id: Uuid,
//! ) -> Result<bool, Box<dyn std::error::Error>> {
//!     let applied = try_apply_event(
//!         tx,
//!         "customer_balance",
//!         "tenant-123",
//!         event_id,
//!         Utc::now(),
//!         |tx| Box::pin(async move {
//!             // Apply event to read model here
//!             sqlx::query("UPDATE balances SET amount = amount + $1")
//!                 .bind(100)
//!                 .execute(tx)
//!                 .await?;
//!             Ok(())
//!         })
//!     ).await?;
//!
//!     Ok(applied)
//! }
//! ```

pub mod admin;
pub mod cursor;
pub mod digest;
pub mod fallback;
pub mod metrics;
pub mod rebuild;
pub mod validate;

// Re-export main types for convenience
pub use cursor::{try_apply_event, CursorError, CursorResult, ProjectionCursor};
pub use digest::{compute_versioned_digest, VersionedDigest, DIGEST_VERSION};
pub use fallback::{
    CircuitBreaker, FallbackError, FallbackMetrics, FallbackPolicy, FallbackResult,
};
pub use rebuild::{
    compute_digest, create_shadow_cursor_table, create_shadow_table, drop_shadow_table,
    load_shadow_cursor, save_shadow_cursor, swap_cursor_tables_atomic, swap_tables_atomic,
    RebuildError, RebuildResult, RebuildSummary,
};
pub use validate::{
    validate_order_column, validate_projection_name, ValidationError, ALLOWED_ORDER_COLUMNS,
    ALLOWED_PROJECTION_TABLES,
};
