#![allow(dead_code)]
//! Failure Injection Model for Simulation (bd-3c2)
//!
//! **Purpose:** Deterministic failure scenarios for testing resilience
//!
//! **ChatGPT Requirements:**
//! - Declines
//! - Timeouts → UNKNOWN
//! - Duplicate webhook deliveries
//! - Late webhooks
//! - Replay of prior events

use crate::seed::{FailureType, SimulationSeed};

/// Failure injection coordinator
pub struct FailureInjector {
    /// Deterministic seed for reproducibility
    seed: SimulationSeed,
}

impl FailureInjector {
    /// Create new failure injector with seed
    pub fn new(seed: SimulationSeed) -> Self {
        Self { seed }
    }

    /// Determine payment outcome for tenant/cycle
    ///
    /// **Returns:** FailureType indicating what should happen
    pub fn determine_payment_outcome(&mut self) -> PaymentOutcome {
        let failure_type = self.seed.generate_failure_type();

        match failure_type {
            FailureType::Decline => PaymentOutcome::Decline {
                code: "card_declined".to_string(),
                message: "Insufficient funds".to_string(),
            },
            FailureType::Timeout => PaymentOutcome::Timeout {
                duration_ms: 5000,
                should_transition_to_unknown: true,
            },
            FailureType::Success => PaymentOutcome::Success {
                psp_reference: format!("psp-{}", uuid::Uuid::new_v4()),
            },
        }
    }

    /// Should this webhook be duplicated?
    ///
    /// **ChatGPT Requirement:** Duplicate webhook deliveries
    pub fn should_duplicate_webhook(&mut self) -> bool {
        self.seed.should_duplicate_webhook()
    }

    /// Should this event be replayed?
    ///
    /// **ChatGPT Requirement:** Replay of prior events
    pub fn should_replay_event(&mut self) -> bool {
        self.seed.should_replay_event()
    }

    /// Get worker count for this cycle
    ///
    /// **ChatGPT Requirement:** 8-32 workers, barrier start
    pub fn get_worker_count(&mut self) -> usize {
        self.seed.generate_worker_count()
    }

    /// Determine webhook delivery delay (0ms for immediate, >0 for late)
    ///
    /// **ChatGPT Requirement:** Late webhooks
    pub fn determine_webhook_delay_ms(&mut self) -> u64 {
        if self.seed.should_fail(0.1) {
            // 10% chance of late webhook
            self.seed.generate_uuid().as_u128() as u64 % 1000 // 0-1000ms delay
        } else {
            0 // Immediate delivery
        }
    }
}

/// Payment outcome for failure injection
#[derive(Debug, Clone)]
pub enum PaymentOutcome {
    /// Payment declined
    Decline { code: String, message: String },
    /// Payment timeout (→ UNKNOWN)
    Timeout {
        duration_ms: u64,
        should_transition_to_unknown: bool,
    },
    /// Payment success
    Success { psp_reference: String },
}

impl PaymentOutcome {
    /// Check if this outcome should result in UNKNOWN status
    pub fn should_be_unknown(&self) -> bool {
        matches!(
            self,
            PaymentOutcome::Timeout {
                should_transition_to_unknown: true,
                ..
            }
        )
    }

    /// Check if this outcome is a success
    pub fn is_success(&self) -> bool {
        matches!(self, PaymentOutcome::Success { .. })
    }

    /// Check if this outcome is a decline
    pub fn is_decline(&self) -> bool {
        matches!(self, PaymentOutcome::Decline { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_payment_outcomes() {
        let seed = SimulationSeed::new(42);
        let mut injector1 = FailureInjector::new(seed.clone());
        let mut injector2 = FailureInjector::new(seed.clone());

        // Same seed should produce same outcomes
        for _ in 0..10 {
            let outcome1 = injector1.determine_payment_outcome();
            let outcome2 = injector2.determine_payment_outcome();

            match (outcome1, outcome2) {
                (PaymentOutcome::Decline { .. }, PaymentOutcome::Decline { .. }) => {}
                (PaymentOutcome::Timeout { .. }, PaymentOutcome::Timeout { .. }) => {}
                (PaymentOutcome::Success { .. }, PaymentOutcome::Success { .. }) => {}
                _ => panic!("Outcomes don't match for same seed"),
            }
        }
    }

    #[test]
    fn test_payment_outcome_helpers() {
        let decline = PaymentOutcome::Decline {
            code: "card_declined".to_string(),
            message: "Test".to_string(),
        };
        assert!(decline.is_decline());
        assert!(!decline.is_success());
        assert!(!decline.should_be_unknown());

        let timeout = PaymentOutcome::Timeout {
            duration_ms: 5000,
            should_transition_to_unknown: true,
        };
        assert!(timeout.should_be_unknown());
        assert!(!timeout.is_success());

        let success = PaymentOutcome::Success {
            psp_reference: "test".to_string(),
        };
        assert!(success.is_success());
        assert!(!success.should_be_unknown());
    }
}
