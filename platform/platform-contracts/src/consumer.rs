//! # Consumer Dedupe & Replay Invariants
//!
//! Every event consumer on the platform must satisfy these invariants.
//!
//! ## Dedupe Key
//!
//! The dedupe key is **`event_id`** (UUID).  Consumers persist processed
//! event IDs in a `processed_events` table:
//!
//! ```sql
//! CREATE TABLE processed_events (
//!     id           SERIAL PRIMARY KEY,
//!     event_id     UUID NOT NULL UNIQUE,
//!     event_type   VARCHAR(255) NOT NULL,
//!     processed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
//!     processor    VARCHAR(100) NOT NULL
//! );
//! ```
//!
//! ## Processing Algorithm
//!
//! 1. Receive event from NATS.
//! 2. Check `processed_events` for `event_id`.
//!    - If found → **skip** (already processed, safe no-op).
//! 3. Process the event (apply business logic).
//! 4. Within the **same transaction** that applies changes, INSERT into
//!    `processed_events`.
//! 5. Commit.
//!
//! Steps 3-5 **must** be atomic (single DB transaction) to prevent the
//! scenario where the event is applied but the dedupe record is lost,
//! leading to double-processing on replay.
//!
//! ## Replay Semantics
//!
//! - Replaying an already-processed event is a **no-op** (dedupe catches it).
//! - Events marked `replay_safe: false` must be handled with extra care:
//!   external side effects (email, SMS, webhook) must not fire again.
//!   Consumers should check the `replay_safe` flag before triggering
//!   side effects.
//! - JetStream redelivery is treated identically to replay — the consumer
//!   does not distinguish between a deliberate replay and a redelivery.
//!
//! ## Ordering
//!
//! Consumers must **not** assume ordered delivery.  If ordering matters
//! (e.g. state machine transitions), use `causation_id` and/or
//! `occurred_at` to reconstruct order, and reject or defer out-of-order
//! events.

/// Processor name constant for platform-level consumers.
///
/// Module-specific consumers should use their module name
/// (e.g. `"ar"`, `"notifications"`).
pub const PLATFORM_PROCESSOR: &str = "platform";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processor_name_is_stable() {
        assert_eq!(PLATFORM_PROCESSOR, "platform");
    }
}
