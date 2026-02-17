//! Deterministic RNG for demo-seed
//!
//! Uses ChaCha8Rng (smallest Chacha variant) seeded from a u64.
//! Same seed → identical sequence of values across all runs.

use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// Deterministic demo seed backed by ChaCha8Rng
pub struct DemoSeed {
    pub seed: u64,
    rng: ChaCha8Rng,
}

impl DemoSeed {
    /// Create a new seed from a u64
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            rng: ChaCha8Rng::seed_from_u64(seed),
        }
    }

    /// Generate a deterministic correlation ID for a resource.
    /// Format: "{tenant}-{resource_type}-{seed}-{idx}"
    pub fn correlation_id(&mut self, tenant: &str, resource_type: &str, idx: usize) -> String {
        format!("{}-{}-{}-{}", tenant, resource_type, self.seed, idx)
    }

    /// Generate a random invoice amount in cents within [min, max]
    pub fn amount_cents(&mut self, min: i32, max: i32) -> i32 {
        self.rng.gen_range(min..=max)
    }

    /// Generate a random due-date offset in days within [min, max]
    pub fn due_days(&mut self, min: u32, max: u32) -> u32 {
        self.rng.gen_range(min..=max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_produces_same_sequence() {
        let mut s1 = DemoSeed::new(42);
        let mut s2 = DemoSeed::new(42);

        for i in 0..10 {
            assert_eq!(
                s1.amount_cents(1000, 50000),
                s2.amount_cents(1000, 50000),
                "Mismatch at index {i}"
            );
        }
    }

    #[test]
    fn different_seeds_produce_different_sequences() {
        let mut s1 = DemoSeed::new(42);
        let mut s2 = DemoSeed::new(99);

        let seq1: Vec<i32> = (0..10).map(|_| s1.amount_cents(1000, 50000)).collect();
        let seq2: Vec<i32> = (0..10).map(|_| s2.amount_cents(1000, 50000)).collect();

        assert_ne!(seq1, seq2, "Different seeds should produce different sequences");
    }

    #[test]
    fn correlation_id_is_deterministic() {
        let mut s = DemoSeed::new(42);
        let id = s.correlation_id("t1", "invoice", 5);
        assert_eq!(id, "t1-invoice-42-5");
    }

    #[test]
    fn amount_cents_in_range() {
        let mut s = DemoSeed::new(1);
        for _ in 0..100 {
            let amount = s.amount_cents(1000, 50000);
            assert!(amount >= 1000, "amount {amount} below min 1000");
            assert!(amount <= 50000, "amount {amount} above max 50000");
        }
    }
}
