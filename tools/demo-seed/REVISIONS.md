# demo-seed Revisions

## v1.0.0 — 2026-03-28

Initial proven release. Deterministic demo data seeding for development and testing.

- 104 passing tests covering all seed modules (GL, inventory, BOM, production, party, numbering)
- Deterministic RNG via ChaCha8Rng — same seed always produces identical data
- Dataset digest tracking for reproducibility verification
- Proof script: `scripts/proof_demo_seed.sh`
