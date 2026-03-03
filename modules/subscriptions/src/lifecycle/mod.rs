//! Subscription Lifecycle Guards and Transition Functions
//!
//! This module owns all lifecycle-critical mutations for subscription status.
//! All status updates MUST route through this module's functions.
//!
//! # State Machine
//! ```text
//! ACTIVE ──> PAST_DUE ──> SUSPENDED
//!   ^    └───────────────────┘  |
//!   └───────────────────────────┘
//! ```
//!
//! # Critical Invariants
//! - Guards validate transitions only (zero side effects)
//! - Side effects occur AFTER guard approval
//! - Pattern: Guard → Mutation → Side Effect

pub mod state_machine;
pub mod transitions;

#[allow(unused_imports)]
pub use state_machine::{transition_guard, SubscriptionStatus, TransitionError};
#[allow(unused_imports)]
pub use transitions::{transition_to_active, transition_to_past_due, transition_to_suspended};
