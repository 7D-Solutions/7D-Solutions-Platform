# simulation

Deterministic multi-tenant simulation harness for billing lifecycle stress and oracle validation.
Runs seeded scenarios with failure injection and reproducibility checks.

Run:
```bash
cargo run -p simulation -- --seed 42 --runs 5 --tenants 15 --cycles 6
```

Config: module database/event-bus connectivity through environment variables.
