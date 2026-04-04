#![allow(dead_code)]
//! Subscriptions module library interface
//!
//! This module provides public APIs for the subscriptions service,
//! including lifecycle management for subscription states and cycle gating.

pub mod admin;
pub mod admin_types;
pub mod bill_run_service;
pub mod config;
pub mod consumer;
pub mod cycle_gating;
pub mod db;
pub mod envelope;
pub mod gated_invoice_creation;
pub mod http;
pub mod invariants;
pub mod lifecycle;
pub mod metrics;
pub mod models;
pub mod outbox;

// Re-export config types for testing
pub use config::Config;

// Re-export commonly used types
pub use lifecycle::{
    transition_guard, transition_to_active, transition_to_past_due, transition_to_suspended,
    SubscriptionStatus, TransitionError,
};

pub use cycle_gating::{
    acquire_cycle_lock, calculate_cycle_boundaries, cycle_attempt_exists, generate_cycle_key,
    mark_attempt_failed, mark_attempt_succeeded, record_cycle_attempt, CycleGatingError,
};

/// Subscriptions application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::SubscriptionsMetrics>,
}
