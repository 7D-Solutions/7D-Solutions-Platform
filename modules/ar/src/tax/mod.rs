//! Tax Provider Interface (bd-8zm)
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
//! responses (bd-29j). The provider may be called at most once per
//! invoice; subsequent reads use the cached response.

pub mod cache;
pub mod jurisdiction;
pub mod models;
pub mod providers;

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

use thiserror::Error;

// ============================================================================
// Error type
// ============================================================================

#[derive(Debug, Error)]
pub enum TaxProviderError {
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("commit rejected: {0}")]
    CommitRejected(String),
    #[error("void rejected: {0}")]
    VoidRejected(String),
    #[error("provider error: {0}")]
    Provider(String),
}

// ============================================================================
// TaxProvider trait
// ============================================================================

/// Provider-agnostic tax interface.
///
/// Implementations: Avalara, TaxJar, local-tax (bd-29j), etc.
///
/// Implementors MUST be `Send + Sync` (shared across Tokio tasks).
/// All methods are async and MUST NOT block.
///
/// Error handling: providers should return `TaxProviderError::Unavailable`
/// for transient failures so callers can apply retry/circuit-breaker logic.
pub trait TaxProvider: Send + Sync {
    fn quote_tax(
        &self,
        req: TaxQuoteRequest,
    ) -> impl std::future::Future<Output = Result<TaxQuoteResponse, TaxProviderError>> + Send;

    fn commit_tax(
        &self,
        req: TaxCommitRequest,
    ) -> impl std::future::Future<Output = Result<TaxCommitResponse, TaxProviderError>> + Send;

    fn void_tax(
        &self,
        req: TaxVoidRequest,
    ) -> impl std::future::Future<Output = Result<TaxVoidResponse, TaxProviderError>> + Send;
}
