//! Reconciliation Matching Engine v1 — Deterministic Heuristics (bd-2cn)
//!
//! ## Matching Strategy
//!
//! The engine proposes matches between unmatched payments (charges with
//! status='succeeded') and open invoices using stable, deterministic rules:
//!
//! 1. **Exact match**: same customer + same amount + same currency → confidence 1.0
//! 2. **Reference match**: external reference ID correlation → confidence 0.95
//! 3. Payments left unmatched after all rules → exception (UnmatchedPayment)
//! 4. Multiple invoices match with equal score → exception (AmbiguousMatch)
//!
//! ## Invariants
//!
//! - **Deterministic**: same inputs always produce same matches across runs.
//! - **Append-only**: match decisions are immutable. Raw inputs are never mutated.
//! - **Atomic**: match persistence + outbox event in a single transaction.
//! - **Idempotent**: duplicate `recon_run_id` returns existing run without error.

mod engine;
mod matching;

// Re-export public API
pub use engine::run_reconciliation;
pub use matching::exception_kind_to_str;

use crate::events::ReconExceptionKind;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

// ============================================================================
// Request / Response types
// ============================================================================

/// Request to execute a reconciliation matching run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReconRequest {
    /// Stable ID for this reconciliation run (idempotency anchor).
    pub recon_run_id: Uuid,
    /// Tenant identifier.
    pub app_id: String,
    /// Distributed trace correlation ID.
    pub correlation_id: String,
    /// Causation ID (event/action that triggered this run).
    pub causation_id: Option<String>,
}

/// Result of a reconciliation run.
#[derive(Debug, Clone, Serialize)]
pub struct ReconRunResult {
    pub recon_run_id: Uuid,
    pub status: String,
    pub payment_count: i32,
    pub invoice_count: i32,
    pub match_count: i32,
    pub exception_count: i32,
}

/// Result of checking for an existing run (idempotency).
#[derive(Debug, Clone)]
pub enum RunReconOutcome {
    /// New run executed.
    Executed(ReconRunResult),
    /// Run already exists (idempotency).
    AlreadyExists(ReconRunResult),
}

// ============================================================================
// Error types
// ============================================================================

#[derive(Debug)]
pub enum ReconError {
    /// Database error.
    DatabaseError(String),
    /// Serialization error.
    SerializationError(String),
}

impl fmt::Display for ReconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            Self::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
        }
    }
}

impl std::error::Error for ReconError {}

impl From<sqlx::Error> for ReconError {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e.to_string())
    }
}

// ============================================================================
// Internal row types
// ============================================================================

/// An unmatched payment (charge with status succeeded, no prior match).
#[derive(Debug, Clone)]
pub(crate) struct UnmatchedPayment {
    pub(crate) charge_id: i32,
    pub(crate) ar_customer_id: i32,
    pub(crate) amount_cents: i32,
    pub(crate) currency: String,
    pub(crate) reference_id: Option<String>,
}

/// An open invoice available for matching.
#[derive(Debug, Clone)]
pub(crate) struct OpenInvoice {
    pub(crate) invoice_id: i32,
    pub(crate) ar_customer_id: i32,
    pub(crate) amount_cents: i32,
    pub(crate) currency: String,
    pub(crate) tilled_invoice_id: String,
}

/// A proposed match from the matching engine.
#[derive(Debug, Clone)]
pub(crate) struct ProposedMatch {
    pub(crate) payment: UnmatchedPayment,
    pub(crate) invoice: OpenInvoice,
    pub(crate) matched_amount_minor: i64,
    pub(crate) confidence_score: f64,
    pub(crate) match_method: String,
}

/// A proposed exception from the matching engine.
#[derive(Debug, Clone)]
pub(crate) struct ProposedException {
    pub(crate) payment_id: Option<String>,
    pub(crate) invoice_id: Option<String>,
    pub(crate) exception_kind: ReconExceptionKind,
    pub(crate) description: String,
    pub(crate) amount_minor: Option<i64>,
    pub(crate) currency: Option<String>,
}

// ============================================================================
// SQLx row mappings
// ============================================================================

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for UnmatchedPayment {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            charge_id: row.try_get("charge_id")?,
            ar_customer_id: row.try_get("ar_customer_id")?,
            amount_cents: row.try_get("amount_cents")?,
            currency: row.try_get("currency")?,
            reference_id: row.try_get("reference_id")?,
        })
    }
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for OpenInvoice {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            invoice_id: row.try_get("invoice_id")?,
            ar_customer_id: row.try_get("ar_customer_id")?,
            amount_cents: row.try_get("amount_cents")?,
            currency: row.try_get("currency")?,
            tilled_invoice_id: row.try_get("tilled_invoice_id")?,
        })
    }
}
