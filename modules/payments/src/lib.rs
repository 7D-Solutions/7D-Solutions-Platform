pub mod config;
pub mod consumer_task;
pub mod envelope_validation;
pub mod events;
pub mod handlers;
pub mod http;
pub mod idempotency_keys;
pub mod invariants;
pub mod lifecycle;
pub mod metrics;
pub mod models;
pub mod processor;
pub mod reconciliation;
pub mod retry;
pub mod webhook_handler;
pub mod webhook_signature;

// Re-export config types
pub use config::{Config, PaymentsProvider};

use std::sync::Arc;

/// Payments application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    /// Payment processor, selected at startup via PAYMENTS_PROVIDER config
    pub processor: Arc<dyn processor::PaymentProcessor>,
    /// Tilled API key (set when PAYMENTS_PROVIDER=tilled)
    pub tilled_api_key: Option<String>,
    /// Tilled account ID (set when PAYMENTS_PROVIDER=tilled)
    pub tilled_account_id: Option<String>,
    /// Tilled webhook HMAC secret (set when PAYMENTS_PROVIDER=tilled)
    pub tilled_webhook_secret: Option<String>,
    /// Previous Tilled webhook secret — set only during rotation overlap window.
    pub tilled_webhook_secret_prev: Option<String>,
    /// Projection fallback policy — staleness threshold and time budget.
    /// Constructed once; shared across all requests via Arc<AppState>.
    pub fallback_policy: projections::FallbackPolicy,
    /// Fallback metrics — counters/histograms registered in the global prometheus registry.
    /// Constructed once; shared across all requests via Arc<AppState>.
    pub fallback_metrics: projections::FallbackMetrics,
    /// Circuit breaker for the payment projection fallback path.
    /// Constructed once; shared across all requests via Arc<AppState>.
    /// Failure counts accumulate across calls — a per-request breaker would always reset.
    pub circuit_breaker: projections::CircuitBreaker,
}

pub use consumer_task::start_payment_collection_consumer;
pub use events::{enqueue_event, EventConsumer, EventEnvelope};
pub use handlers::handle_payment_collection_requested;
pub use models::{PaymentCollectionRequestedPayload, PaymentSucceededPayload};
pub use processor::test_support::TestPaymentProcessor;
pub use processor::{PaymentProcessor, TilledPaymentProcessor};
pub use reconciliation::{
    reconcile_unknown_attempt, PspPaymentStatus, ReconciliationError, ReconciliationResult,
};
pub use retry::{
    calculate_retry_windows, determine_current_window, get_payments_for_retry,
    is_eligible_for_retry, RetryError,
};
