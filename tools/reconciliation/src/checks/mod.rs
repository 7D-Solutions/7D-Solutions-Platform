//! Per-module invariant check functions.
//!
//! Each module function returns a list of [`Violation`] records.
//! An empty list means all invariants passed.
//!
//! Design rules:
//! - Every check runs a single SQL query against live data.
//! - NULL is handled explicitly (COALESCE); no implicit coercion.
//! - Timezone differences are avoided by comparing at UTC (TIMESTAMP WITH TIME ZONE columns).
//! - Each check is independent; a DB error in one does not abort others.

pub mod ap;
pub mod ar;
pub mod bom;
pub mod gl;
pub mod inventory;
pub mod production;

/// A single invariant violation detected during reconciliation.
#[derive(Debug, Clone)]
pub struct Violation {
    /// Module name: ar, ap, gl, inventory, bom, production
    pub module: String,
    /// Short invariant identifier matching the Prometheus label (snake_case).
    pub invariant: String,
    /// Number of rows that violate this invariant.
    pub count: i64,
    /// Human-readable description of the first offending record (for logs).
    pub detail: String,
}

impl Violation {
    pub fn new(module: &str, invariant: &str, count: i64, detail: impl Into<String>) -> Self {
        Self {
            module: module.to_string(),
            invariant: invariant.to_string(),
            count,
            detail: detail.into(),
        }
    }
}
