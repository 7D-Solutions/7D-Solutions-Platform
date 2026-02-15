//! Subscriptions module library interface
//!
//! This module provides public APIs for the subscriptions service,
//! including lifecycle management for subscription states and cycle gating.

pub mod cycle_gating;
pub mod lifecycle;
pub mod models;

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
