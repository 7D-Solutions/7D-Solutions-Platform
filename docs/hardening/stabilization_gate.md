# Stabilization Gate — Thresholds & Baseline

This document defines the constitutional thresholds for the stabilization gate harness
(`tools/stabilization-gate`). All thresholds are treated as hard gates: violation = FAIL.

## Hardware Assumptions

The gate was baselined on developer hardware and scaled conservatively:

| Parameter | Baseline value |
|-----------|---------------|
| CPU cores | 8 (Apple M-series or equivalent x86-64) |
| Memory | 16 GB |
| Postgres | local, single-node, default config |
| NATS | local, single-node, no persistence |
| Network | loopback only (no cross-host latency) |

**Production note:** Production machines will meet or exceed these specs. Thresholds are
set conservatively so they pass on a developer laptop and are trivially exceeded in
production. Raise them only after a sustained production baseline has been collected.

---

## Scenario Thresholds

### 1. Event Bus (`eventbus`)

Measures NATS publish/consume throughput and end-to-end delivery latency under
concurrent multi-tenant load with real `EventEnvelope` serialization.

| Metric | Env var | Default | Interpretation |
|--------|---------|---------|----------------|
| Minimum publish throughput | `EVENTBUS_MIN_THROUGHPUT` | 500 events/sec | Sustained publish rate |
| Maximum P99 latency | `EVENTBUS_MAX_P99_MS` | 500 ms | End-to-end delivery |
| Maximum drop count | `EVENTBUS_MAX_DROP_COUNT` | 0 | Zero drop tolerance |

**Invariant (hard-coded):** `sent_count == recv_count`. Any discrepancy is a violation
regardless of the drop count threshold.

---

### 2. Projections (`projections`)

Two-phase benchmark:
- **Rebuild phase:** Blue/green shadow swap for three projection types:
  `invoice_summary`, `customer_balance`, `subscription_status`.
- **Lag phase:** Sustained NATS publish → DB cursor write; measures publish→DB-write lag.

| Metric | Env var | Default | Interpretation |
|--------|---------|---------|----------------|
| Maximum rebuild duration per projection | `PROJ_MAX_REBUILD_SECS` | 300 s | Full projection rebuild |
| Maximum end-to-end lag | `PROJ_MAX_LAG_MS` | 2000 ms | NATS publish → DB cursor write |

---

### 3. AR Reconciliation (`recon`)

Seeds charge/invoice pairs per tenant, runs `run_reconciliation()` concurrently,
validates 1:1 exact matching.

| Metric | Env var | Default | Interpretation |
|--------|---------|---------|----------------|
| Minimum match throughput | `RECON_MIN_MATCHES_PER_SEC` | 50 matches/sec | Across all tenants |
| Maximum exception rate | `RECON_MAX_EXCEPTION_RATE` | 0.05 (5%) | Unmatched rows |

**Invariant (hard-coded):** No `payment_id` in `ar_recon_matches` appears more than once
per tenant (`app_id`). Duplicate matches = FAIL regardless of threshold.

---

### 4. Dunning (`dunning`)

Seeds overdue invoices per tenant, runs `init_dunning + transition_dunning`
(Pending → Warned) cycles concurrently.

| Metric | Env var | Default | Interpretation |
|--------|---------|---------|----------------|
| Minimum throughput | `DUNNING_MIN_THROUGHPUT_PER_SEC` | 50 rows/sec | Init + transition per row |
| Maximum drain time | `DUNNING_MAX_DRAIN_SECS` | 300 s | Wall-clock for all rows |

**Invariant (hard-coded):** After exactly 1 init + 1 transition, `version = 2`. Any row
with `version > 2` indicates duplicate processing = FAIL.

---

### 5. Cross-Tenant Isolation (`cross_tenant_isolation`)

Validates that parallel execution produced no cross-tenant data contamination.

| Check | Threshold | Query |
|-------|-----------|-------|
| Recon bleed | 0 | `payment_id` shared across multiple `bench-rc-*` app\_ids |
| Dunning bleed | 0 | Dunning state references invoice owned by different `app_id` |

**Both checks are hard-coded to zero tolerance.** Any contamination = FAIL, non-configurable.

---

### 6. DB Connectivity (`projections` + `tenants` stubs)

Used in benchmarks.rs (Wave 0 connectivity checks):

| Metric | Threshold |
|--------|-----------|
| DB ping P99 latency | 200 ms |

---

## Default Load Parameters

These defaults are used when no env vars are set. Scaled for a developer laptop:

| Env var | Default | Purpose |
|---------|---------|---------|
| `TENANT_COUNT` | 5 | Simulated tenants |
| `EVENTS_PER_TENANT` | 100 | Events published per tenant |
| `RECON_ROWS` | 500 | Total reconciliation rows |
| `DUNNING_ROWS` | 200 | Total dunning rows |
| `CONCURRENCY` | 4 | Worker parallelism |
| `DURATION_SECS` | 30 | Drain window for timed scenarios |

For a production stress run, recommended values:

```bash
TENANT_COUNT=25 \
EVENTS_PER_TENANT=200 \
RECON_ROWS=2000 \
DUNNING_ROWS=1000 \
CONCURRENCY=50 \
DURATION_SECS=120 \
  cargo run -p stabilization-gate -- run-all
```

---

## Baseline Report

A committed baseline report lives at:

```
tools/stabilization-gate/reports/baseline.json
```

This represents a known-good run against a developer workstation.
CI dry-run mode (`--dry-run`) verifies connectivity only and always passes as long as
Postgres and NATS are reachable. Full benchmarks are run in nightly or pre-release CI.

---

## Running the Gate

### Dry-run (connectivity check only — used in CI):
```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/ar_db \
NATS_URL=nats://localhost:4222 \
  cargo run -p stabilization-gate -- run-all --dry-run
```

### Full benchmark (all thresholds enforced):
```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5434/ar_db \
  cargo run -p stabilization-gate -- run-all
```

### Individual scenarios:
```bash
cargo run -p stabilization-gate -- eventbus
cargo run -p stabilization-gate -- projections
cargo run -p stabilization-gate -- recon
cargo run -p stabilization-gate -- dunning
```

---

## CI Integration

The gate runs in dry-run mode on every push/PR via `.github/workflows/hardening.yml`
(`stabilization-gate-dry` job). This verifies:
1. The binary compiles.
2. Postgres and NATS are reachable.
3. No threshold enforcement (dry-run only tests connectivity).

A full benchmark run should be scheduled as a nightly job once the production baseline
is established.
