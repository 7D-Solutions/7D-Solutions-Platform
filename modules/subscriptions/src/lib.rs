//! Subscriptions module library interface
//!
//! This module provides public APIs for the subscriptions service,
//! including lifecycle management for subscription states.

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
