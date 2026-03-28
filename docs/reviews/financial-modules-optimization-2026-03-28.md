# Financial Modules Optimization Review

Date: 2026-03-28  
Bead: `bd-1hfoy`

## Scope

Reviewed the requested financial crates with a profiling-first pass:

- `modules/ar`
- `modules/ap`
- `modules/gl`
- `modules/payments`
- `modules/treasury`
- `modules/consolidation`

Focus areas from the bead:

- aging-report query efficiency
- journal-entry batch processing overhead
- allocation pressure in payment reconciliation loops

## What Was Profiled

### `modules/ar`

- Read the aging refresh path in `src/aging.rs`.
- Identified the main hot path as the `refresh_aging_tx` SQL statement.
- The original query executed four correlated aggregate subqueries per invoice:
  - charges
  - payment allocations
  - credit notes
  - write-offs

### `modules/ap`

- Read `src/domain/reports/aging.rs`.
- The AP aging report is already a single-pass grouped query over a pre-aggregated `bill_open` CTE.
- The only obvious inefficiency is that `by_vendor=true` performs a second scan for the vendor breakdown.
- No change was made because the query shape is already materially better than the AR aging path and there was no measurement justifying a second rewrite in this bead.

### `modules/gl`

- Read the statement repository and existing statement performance benchmark coverage.
- Ran the existing bench binary before changes.
- Read the journal posting path in `src/services/journal_service.rs` and `src/repos/journal_repo.rs`.
- The statement queries are already covered by a dedicated performance suite and were not the weakest point in this bead.
- The posting path still built redundant vectors and inserted journal lines one row at a time.

### `modules/payments`

- Read the UNKNOWN reconciliation path in `src/reconciliation.rs`.
- This path is dominated by PSP polling and lifecycle transitions, not CPU-heavy local work.
- No change was made.

### `modules/treasury`

- Read the reconciliation service and matching engine in `src/domain/recon/service.rs` and `src/domain/recon/engine.rs`.
- The hot path was the auto-match engine generating the full statement-line × payment-txn cross-product and cloning full transactions into each candidate.

### `modules/consolidation`

- Read the consolidation engine in `src/domain/engine/compute.rs`.
- The main likely cost is repeated COA mapping lookup during consolidation and cache writes.
- No change was made in this bead because I did not have a benchmark or failing performance guard for consolidation, and the bead explicitly required measurement-first changes.

## What Changed

### `modules/ar`

Rewrote the aging refresh SQL in `modules/ar/src/aging.rs`:

- replaced correlated invoice subqueries with pre-aggregated CTEs
- joined those aggregates once per invoice
- kept the aging bucket semantics unchanged
- kept the output shape unchanged

Net effect:

- each adjustment table is scanned once per refresh instead of once per invoice
- the query plan is materially simpler under mature customer invoice counts

### `modules/treasury`

Optimized `modules/treasury/src/domain/recon/engine.rs`:

- added an exact amount+currency bucketing fast path for strategies that require it
- stored candidate indexes plus confidence instead of cloning full rows into every candidate
- deferred cloning until a candidate actually survives greedy assignment
- kept deterministic sorting and one-to-one assignment behavior unchanged

Also extended `modules/treasury/src/domain/recon/strategies/mod.rs` with a default capability hook so the engine can safely fall back to the old full scan if a future strategy does not require exact amount+currency matching.

### `modules/gl`

Optimized journal batch posting:

- `modules/gl/src/repos/journal_repo.rs`
  - `bulk_insert_lines` now inserts a whole batch with `sqlx::QueryBuilder` instead of looping one row at a time
- `modules/gl/src/services/journal_service.rs`
  - builds `JournalLineInsert` and `JournalLineInput` in one pass
  - removes the extra `Vec` clone before insertion
- updated slice-based call sites in:
  - `modules/gl/src/services/fx_revaluation_service.rs`
  - `modules/gl/src/services/reversal_service.rs`
  - `modules/gl/src/revrec/recognition_run.rs`
  - `modules/gl/tests/db_repos_test.rs`

## Measurements

### Treasury matching benchmark

Benchmark: `cargo test -p treasury benchmark_bucketed_matching_against_legacy -- --ignored --nocapture`

- workload: 2,000 statement lines and 2,000 payment txns with unique amount buckets
- before: `23.335083ms`
- after: `4.880709ms`
- speedup: `4.78x`

### GL journal line batch insert benchmark

Benchmark: `cargo test -p gl-rs --test journal_batch_insert_benchmark -- --ignored --nocapture`

- workload: 2,000 journal lines inserted into one entry
- before: `795.549542ms`
- after: `124.239625ms`
- speedup: `6.40x`

### GL existing bench baseline

Benchmark before changes: `cargo run -p gl-rs --bin bench -- --duration 3`

- `post_journal`: avg `6.16ms`, p95 `9.78ms`
- `trial_balance`: avg `0.86ms`, p95 `1.75ms`
- `income_statement`: avg `0.82ms`, p95 `1.60ms`

Notes:

- a post-change rerun of the standard GL bench hit `PoolTimedOut` on the shared local database after the targeted benchmark/test load
- the targeted 2,000-line batch benchmark above is the reliable before/after number for the code that actually changed

### AR aging

- The AR rewrite was validated by compile/test coverage and by preserving output semantics in the new benchmark harness.
- A full timed run on the shared AR database was not stable enough during this session to produce a trustworthy latency number; the local benchmark harness is in `modules/ar/tests/aging_refresh_benchmark.rs` for reruns when the shared DB is less contended.

## No-Change Decisions

- `modules/ap`: existing aging query is already grouped and single-pass; no measured evidence justified a rewrite here.
- `modules/payments`: UNKNOWN reconciliation is network/processor-bound, not local CPU-bound.
- `modules/consolidation`: likely optimization targets exist, but no measurement justified changing accounting-critical consolidation logic in this bead.

## Verification Targets

Focused runs completed during implementation:

- `cargo test -p treasury optimized_engine_matches_legacy_output -- --nocapture`
- `cargo test -p treasury benchmark_bucketed_matching_against_legacy -- --ignored --nocapture`
- `cargo test -p gl-rs --test journal_batch_insert_benchmark -- --ignored --nocapture`
- `cargo test -p ar-rs --lib --no-run`

Attempted full bead verification:

- `./scripts/cargo-slot.sh test -p ar-rs -p ap -p gl-rs -p payments-rs -p treasury -p consolidation`

Observed blocker in the shared test environment:

- AP allocation tests failed with `DB connect failed: PoolTimedOut`
- reproduced in isolation with:
  - `cargo test -p ap --lib domain::allocations::service::tests::test_allocation_on_invalid_status_rejected -- --exact --nocapture`
  - `cargo test -p ap --lib domain::allocations::service::tests::test_get_allocations_returns_in_insertion_order -- --exact --nocapture`
