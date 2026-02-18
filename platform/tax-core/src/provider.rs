//! TaxProvider trait — provider-agnostic tax calculation interface.

use crate::error::TaxProviderError;
use crate::models::{
    TaxCommitRequest, TaxCommitResponse, TaxQuoteRequest, TaxQuoteResponse, TaxVoidRequest,
    TaxVoidResponse,
};

/// Provider-agnostic tax interface.
///
/// Implementations include Avalara, TaxJar, and local/zero-tax adapters.
///
/// ## Lifecycle
///
/// ```text
/// quote_tax  → provider calculates tax for an invoice draft (not committed)
/// commit_tax → provider commits tax when invoice is finalized
/// void_tax   → provider voids committed tax on refund/write-off
/// ```
///
/// ## Requirements
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
