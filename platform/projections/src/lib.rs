//! Platform-level projection infrastructure
//!
//! This crate provides the foundational types and contracts for projection management:
//! - Cursor tracking for event stream position
//! - Metrics for projection health monitoring
//!
//! This is scaffolding only. Business logic will be implemented in subsequent beads.

pub mod cursor;
pub mod metrics;
