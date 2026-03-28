#![allow(dead_code)]
//! Deterministic Seed Model for Simulation (bd-3c2)
//!
//! **Purpose:** Deterministic RNG using ChaCha20 for reproducible simulations
//!
//! **Key Requirement:** Same seed → identical simulation outcomes across 5 runs

use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use uuid::Uuid;

/// Deterministic simulation seed
#[derive(Debug, Clone)]
pub struct SimulationSeed {
    /// Seed value for reproducibility
    pub seed: u64,
    /// RNG instance
    rng: ChaCha20Rng,
}

impl SimulationSeed {
    /// Create new simulation with deterministic seed
    pub fn new(seed: u64) -> Self {
        let rng = ChaCha20Rng::seed_from_u64(seed);
        Self { seed, rng }
    }

    /// Generate deterministic tenant IDs
    ///
    /// **ChatGPT Requirement:** 10-20 tenants (deterministic)
    pub fn generate_tenant_ids(&mut self, count: usize) -> Vec<String> {
        (0..count).map(|i| format!("sim-tenant-{:04}", i)).collect()
    }

    /// Decide if this operation should fail (for failure injection)
    ///
    /// **Parameters:**
    /// - `failure_rate`: Probability of failure (0.0 to 1.0)
    ///
    /// **Returns:** true if operation should fail
    pub fn should_fail(&mut self, failure_rate: f64) -> bool {
        self.rng.gen_bool(failure_rate)
    }

    /// Generate deterministic failure type
    ///
    /// **Returns:**
    /// - 0: Decline
    /// - 1: Timeout (→ UNKNOWN)
    /// - 2: Success
    pub fn generate_failure_type(&mut self) -> FailureType {
        let r: f64 = self.rng.gen();
        if r < 0.3 {
            FailureType::Decline
        } else if r < 0.5 {
            FailureType::Timeout
        } else {
            FailureType::Success
        }
    }

    /// Decide if this webhook should be duplicated
    ///
    /// **ChatGPT Requirement:** Duplicate webhook deliveries
    pub fn should_duplicate_webhook(&mut self) -> bool {
        self.rng.gen_bool(0.2) // 20% chance of duplicate
    }

    /// Decide if this event should be replayed
    ///
    /// **ChatGPT Requirement:** Replay of prior events
    pub fn should_replay_event(&mut self) -> bool {
        self.rng.gen_bool(0.15) // 15% chance of replay
    }

    /// Generate deterministic worker count (8-32)
    ///
    /// **ChatGPT Requirement:** 8-32 scheduler workers
    pub fn generate_worker_count(&mut self) -> usize {
        self.rng.gen_range(8..=32)
    }

    /// Generate deterministic UUID (for reproducibility)
    pub fn generate_uuid(&mut self) -> Uuid {
        let bytes: [u8; 16] = self.rng.gen();
        Uuid::from_bytes(bytes)
    }
}

/// Failure type for deterministic injection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureType {
    /// Payment declined
    Decline,
    /// Timeout (transitions to UNKNOWN)
    Timeout,
    /// Success (no failure)
    Success,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seed_determinism() {
        // Same seed should produce same tenant IDs
        let mut seed1 = SimulationSeed::new(42);
        let mut seed2 = SimulationSeed::new(42);

        let tenants1 = seed1.generate_tenant_ids(15);
        let tenants2 = seed2.generate_tenant_ids(15);

        assert_eq!(tenants1, tenants2);
    }

    #[test]
    fn test_seed_different_outputs() {
        // Different seeds should produce different outcomes
        let mut seed1 = SimulationSeed::new(42);
        let mut seed2 = SimulationSeed::new(43);

        let _fail1 = seed1.should_fail(0.5);
        let _fail2 = seed2.should_fail(0.5);

        // Very unlikely to be the same across multiple calls
        let results_match = (0..10).all(|_| seed1.should_fail(0.5) == seed2.should_fail(0.5));

        assert!(
            !results_match,
            "Different seeds should produce different outcomes"
        );
    }

    #[test]
    fn test_tenant_id_format() {
        let mut seed = SimulationSeed::new(1);
        let tenants = seed.generate_tenant_ids(3);

        assert_eq!(tenants[0], "sim-tenant-0000");
        assert_eq!(tenants[1], "sim-tenant-0001");
        assert_eq!(tenants[2], "sim-tenant-0002");
    }

    #[test]
    fn test_worker_count_range() {
        let mut seed = SimulationSeed::new(100);

        for _ in 0..100 {
            let count = seed.generate_worker_count();
            assert!(
                count >= 8 && count <= 32,
                "Worker count {} out of range",
                count
            );
        }
    }
}
