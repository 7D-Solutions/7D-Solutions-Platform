//! Tax Provider Interface
//!
//! Defines a provider-agnostic `TaxProvider` trait that Avalara, TaxJar,
//! and local-tax adapters will implement. AR invoice calculation paths
//! call this interface without knowing the underlying provider.
//!
//! ## Lifecycle
//!
//! ```text
//! quote_tax  → provider calculates tax for an invoice draft
//! commit_tax → provider commits tax when invoice is finalized
//! void_tax   → provider voids committed tax on refund/write-off
//! ```
//!
//! ## Determinism
//!
//! Tax calculations MUST be deterministic when using cached provider
//! responses. The provider may be called at most once per invoice;
//! subsequent reads use the cached response.

pub mod cache;
pub mod jurisdiction;
pub mod models;
pub mod providers;
pub mod reporting;

// Re-export all public types for backward-compatible `crate::tax::Foo` paths
pub use cache::{
    compute_request_hash, find_cached_quote, quote_tax_cached, store_quote_cache, CachedTaxQuote,
};
pub use jurisdiction::{
    compute_resolution_hash, get_jurisdiction_snapshot, insert_jurisdiction, insert_tax_rule,
    persist_jurisdiction_snapshot, resolve_and_persist_tax, resolve_jurisdiction,
};
pub use models::*;
pub use providers::{LocalTaxProvider, ZeroTaxProvider};
pub use reporting::{
    render_csv, resolve_stacked_jurisdictions, tax_summary_by_period, TaxSummaryRow,
};

// Re-export shared types from tax-core so callers use `crate::tax::TaxProvider` etc.
pub use tax_core::{TaxProvider, TaxProviderError};
