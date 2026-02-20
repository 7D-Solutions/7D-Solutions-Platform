//! `tax-core` — shared tax types and provider trait for the 7D platform.
//!
//! Provides the foundational types consumed by AR, AP, and any future module
//! that performs tax calculations:
//!
//! - **[`models`]**: request/response value objects and jurisdiction snapshots
//! - **[`provider`]**: [`TaxProvider`] trait for pluggable provider adapters
//! - **[`error`]**: [`TaxProviderError`] returned by provider implementations
//!
//! ## Stability
//!
//! These types form a shared contract. Breaking changes require updating all
//! consuming crates (AR, AP, …). Additive changes (new optional fields) are
//! fine; field removals or renames require a coordinated migration bead.

pub mod error;
pub mod jurisdiction;
pub mod local_tax;
pub mod models;
pub mod provider;
pub mod zero_tax;

// Convenience re-exports so consumers can write `tax_core::TaxProvider` etc.
pub use error::TaxProviderError;
pub use jurisdiction::{JurisdictionConfig, JurisdictionEntry, TaxRuleConfig};
pub use local_tax::LocalTaxProvider;
pub use models::*;
pub use provider::TaxProvider;
pub use zero_tax::ZeroTaxProvider;
