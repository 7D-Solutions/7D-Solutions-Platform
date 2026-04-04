pub mod config;
pub mod consumer_task;
pub mod envelope_validation;
pub mod events;
pub mod handlers;
pub mod idempotency_keys;
pub mod invariants;
pub mod lifecycle;
pub mod metrics;
pub mod models;
pub mod processor;
pub mod reconciliation;
pub mod retry;
pub mod http;
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
}

pub use consumer_task::start_payment_collection_consumer;
pub use events::{enqueue_event, EventConsumer, EventEnvelope};
pub use handlers::handle_payment_collection_requested;
pub use models::{PaymentCollectionRequestedPayload, PaymentSucceededPayload};
pub use processor::{PaymentProcessor, TilledPaymentProcessor};
pub use processor::test_support::TestPaymentProcessor;
pub use reconciliation::{
    reconcile_unknown_attempt, PspPaymentStatus, ReconciliationError, ReconciliationResult,
};
pub use retry::{
    calculate_retry_windows, determine_current_window, get_payments_for_retry,
    is_eligible_for_retry, RetryError,
};
