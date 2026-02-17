//! Subscriptions module library interface
//!
//! This module provides public APIs for the subscriptions service,
//! including lifecycle management for subscription states and cycle gating.

pub mod config;
pub mod cycle_gating;
pub mod outbox;
pub mod gated_invoice_creation;
pub mod invariants;
pub mod lifecycle;
pub mod metrics;
pub mod models;

// Re-export config types for testing
pub use config::Config;

// Re-export commonly used types
pub use lifecycle::{
    SubscriptionStatus,
    TransitionError,
    transition_guard,
    transition_to_active,
    transition_to_past_due,
    transition_to_suspended,
};

pub use cycle_gating::{
    generate_cycle_key,
    calculate_cycle_boundaries,
    acquire_cycle_lock,
    cycle_attempt_exists,
    record_cycle_attempt,
    mark_attempt_succeeded,
    mark_attempt_failed,
    CycleGatingError,
};

/// Subscriptions application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::SubscriptionsMetrics>,
}
