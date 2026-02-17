//! GL event contracts — domain-specific event types and payloads
//!
//! Split into domain files:
//! - `accruals` — accrual creation/reversal events, cash flow classification, reversal policy
//! - `fx` — FX rate updates, unrealized/realized gain/loss postings

pub mod accruals;
pub mod fx;

// ============================================================================
// Shared Mutation Classes
// ============================================================================

/// DATA_MUTATION: creates or modifies a financial record
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

/// REVERSAL: compensates for a prior DATA_MUTATION (accrual reversal)
pub const MUTATION_CLASS_REVERSAL: &str = "REVERSAL";

// Re-export everything so `contracts::*` paths keep working
pub use accruals::*;
pub use fx::*;
