# stabilization-gate

Benchmark harness for production-readiness checks against real Postgres and NATS.
Emits JSON/Markdown reports under `tools/stabilization-gate/reports/`.

Run:
```bash
cargo run -p stabilization-gate -- run-all --tenant-count 25 --events-per-tenant 200
cargo run -p stabilization-gate -- e2e-bench --runs 2
```

Config: benchmark/env settings via CLI flags and environment variables.
