//! # Platform Contracts
//!
//! Canonical conventions for events, commands, and idempotency across the
//! 7D Solutions Platform.  Every service scaffold imports this crate to get
//! shared constants and re-exported types — ensuring no module invents
//! incompatible patterns.
//!
//! ## What lives here
//!
//! | Area | Module |
//! |------|--------|
//! | Event envelope | Re-export of [`event_bus::EventEnvelope`] |
//! | Event naming | [`event_naming`] — format, versioning rules |
//! | Mutation classes | [`mutation_classes`] — the 7 canonical classes |
//! | Idempotency | [`idempotency`] — key format, TTL, replay rules |
//! | Consumer contract | [`consumer`] — dedupe invariants |

pub mod consumer;
pub mod event_naming;
pub mod idempotency;
pub mod mutation_classes;
pub mod portal_identity;

// ── Re-exports from event-bus ───────────────────────────────────────────
pub use event_bus::{EventEnvelope, MerchantContext};
