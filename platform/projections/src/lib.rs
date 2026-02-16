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
//!         |tx| async move {
//!             // Apply event to read model here
//!             sqlx::query("UPDATE balances SET amount = amount + $1")
//!                 .bind(100)
//!                 .execute(tx)
//!                 .await?;
//!             Ok(())
//!         }
//!     ).await?;
//!
//!     Ok(applied)
//! }
//! ```

pub mod cursor;
pub mod metrics;

// Re-export main types for convenience
pub use cursor::{try_apply_event, CursorError, CursorResult, ProjectionCursor};
