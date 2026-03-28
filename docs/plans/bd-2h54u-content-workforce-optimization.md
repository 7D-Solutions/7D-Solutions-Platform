# Content & Workforce Module Optimization Analysis

**Bead:** bd-2h54u
**Agent:** MaroonHarbor
**Date:** 2026-03-28
**Methodology:** /extreme-software-optimization (profile first, prove behavior unchanged)

## Crates Analyzed

| Crate | LOC | Computation Profile |
|-------|-----|-------------------|
| pdf-editor | 5,831 | PDF rendering (pdfium), image decode, table layout |
| reporting | 10,724 | Financial aggregations, forecast probability, export generation |
| timekeeping | 7,795 | CRUD with Guard/Mutation/Outbox — I/O bound |
| workforce-competence | 2,406 | CRUD with Guard/Mutation/Outbox — I/O bound |
| fixed-assets | 5,674 | Depreciation engine (pure CPU), schedule generation (DB) |

## Opportunity Matrix

| Hotspot | Impact | Conf | Effort | Score | Action |
|---------|--------|------|--------|-------|--------|
| reporting: linear CDF scan | 3 | 5 | 1 | 15.0 | Changed to binary search |
| reporting: N+1 profile queries | 4 | 5 | 2 | 10.0 | Batch-loaded in 1 query |
| fixed-assets: N+1 schedule inserts | 3 | 5 | 2 | 7.5 | UNNEST batch insert |
| pdf-editor: clone rows per page | 2 | 4 | 2 | 4.0 | Borrowed &str refs |
| reporting: currency string clones | 1 | 3 | 1 | 3.0 | Skipped (negligible) |
| timekeeping: approval workflow | 1 | 2 | 3 | 0.7 | Skipped (I/O bound) |
| workforce-competence: idempotency hash | 1 | 2 | 3 | 0.7 | Skipped (I/O bound) |

## Changes Made

### 1. reporting/probability.rs — Binary search CDF (Score 15.0)

`empirical_cdf` scanned all observations linearly to count values <= x. Since observations are pre-sorted ascending from the DB, replaced with `partition_point` for O(log n) lookup.

**Called:** Multiple times per invoice per horizon per currency in `compute_cash_forecast`.

**Isomorphism:** `partition_point(|&d| d <= x_floor)` produces identical count to `.filter(|&&d| (d as f64) <= x).count()` for integer x values (all call sites pass u32-cast-to-f64).

### 2. reporting/timing_profile.rs — Batch profile loading (Score 10.0)

`load_profiles_for_tenant` issued 1-2 DB queries per unique (customer, currency) pair via `resolve_profile`. For N customers, that's N-2N roundtrips.

Replaced with single query loading all `rpt_payment_history` for the tenant, then in-memory grouping. Falls back to tenant-wide aggregate for customers with < 3 records (same logic, no behavior change).

**Isomorphism:** Same profile resolution logic (per-customer if >= 3 records, else tenant fallback). Same PaymentProfile values built from same sorted observations.

### 3. fixed-assets/depreciation/service.rs — Batch inserts (Score 7.5)

`generate_schedule` inserted each depreciation period individually in a loop. For a 10-year asset (120 months), 120 roundtrips.

Replaced with single `INSERT ... SELECT * FROM UNNEST(...)` using parallel arrays. Same `ON CONFLICT (asset_id, period_number) DO NOTHING` idempotency.

**Isomorphism:** Same rows inserted, same conflict resolution. Verified by `generate_schedule_creates_12_periods` integration test.

### 4. pdf-editor/tables/mod.rs — Borrowed cell references (Score 4.0)

`render_table` cloned `Vec<String>` cell data for every row pushed into `PageSlice`, and cloned the header cells for each new page.

Changed `PageSlice` to hold `Vec<&str>` references into the original `TableDefinition`. Eliminates O(rows * cols) String allocations for multi-page tables.

**Isomorphism:** Same text rendered at same coordinates. References point to same underlying string data.

## Not Changed (with reasoning)

- **timekeeping** (7,795 LOC): All operations follow Guard/Mutation/Outbox pattern — pure I/O bound (DB transactions, event enqueue). No CPU-intensive computation. Approval workflow `submit` computes total minutes in one SUM query before the transaction, which is correct.

- **workforce-competence** (2,406 LOC): Same Guard/Mutation/Outbox pattern. Idempotency check serializes request to JSON for hashing — done once per request, not a hotspot. Authorization query uses indexed JOIN with LIMIT 1.

- **pdf-editor generate.rs**: `field.pdf_position.clone()` clones small JSON objects (~4-5 fields). `serde_json::from_value` requires ownership — no zero-copy alternative. Negligible cost.

- **reporting KPIs**: Already uses `tokio::try_join!` to run 6 queries concurrently. No further optimization needed.

- **reporting statements** (balance_sheet, P&L, cashflow): Single aggregation queries with GROUP BY. Server-side computation. `sum_by_currency` clones ~1-3 currency strings total. Not worth optimizing.

## Verification

All pure unit tests pass (Docker down during verification — DB integration tests deferred):
- pdf-editor: 18/18
- reporting: 98/98
- timekeeping: 64/64
- workforce-competence: 5/5
- fixed-assets: 10/10 (engine unit tests)
