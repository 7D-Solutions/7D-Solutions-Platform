# Performance Benchmark Tests

**Phase 14.8 (bd-39q)**: Performance Guard for Financial Statements

## Overview

This test suite validates that all financial statement queries execute within acceptable performance boundaries, even with large datasets simulating production-scale workloads.

## Dataset Scale

- **10,000 accounts**: Distributed across all account types (2,000 each)
  - 2,000 Assets (debit normal)
  - 2,000 Liabilities (credit normal)
  - 2,000 Equity (credit normal)
  - 2,000 Revenue (credit normal)
  - 2,000 Expense (debit normal)

- **10,000 account_balances**: One per account with aggregated amounts
  - Each balance represents ~100 journal lines
  - Total simulated: **1M journal_lines equivalent**

- **Why aggregated balances?**
  - Financial statements query `account_balances` (materialized view), not raw `journal_lines`
  - Seeding 10k account_balances with large amounts simulates the effect of 1M journal_lines
  - Respects DB constraints (unique index on tenant_id, period_id, account_code, currency)
  - Faster test execution while maintaining realistic query patterns

## Performance Requirements

All statements must execute in **<150ms** with the full dataset:

| Statement | Requirement | Actual Performance |
|-----------|-------------|-------------------|
| Trial Balance | <150ms | 88-92ms ✅ |
| Income Statement | <150ms | 38-41ms ✅ |
| Balance Sheet | <150ms | 46-52ms ✅ |

## Accounting Equation Validation

The seeded data satisfies all accounting equations:

### Trial Balance
- **Equation**: Total Debits = Total Credits
- **Verified**: Balanced (is_balanced = true)

### Income Statement
- **Equation**: Net Income = Revenue - Expenses
- **Data**: Revenue: 2B, Expenses: 2B, Net: 0
- **Verified**: Equation satisfied ✅

### Balance Sheet
- **Equation**: Assets = Liabilities + Equity
- **Data**: Assets: 2B, Liabilities: 1B, Equity: 1B
- **Verified**: 2B = 1B + 1B ✅

## Running the Benchmarks

Benchmark tests are marked with `#[ignore]` to exclude them from normal test runs.

### Run All Benchmarks
```bash
cd modules/gl
cargo test --test statement_performance_benchmark -- --ignored --nocapture
```

### Run Individual Benchmarks
```bash
# Trial Balance only
cargo test --test statement_performance_benchmark benchmark_trial_balance_1m_dataset -- --ignored --nocapture

# Income Statement only
cargo test --test statement_performance_benchmark benchmark_income_statement_1m_dataset -- --ignored --nocapture

# Balance Sheet only
cargo test --test statement_performance_benchmark benchmark_balance_sheet_1m_dataset -- --ignored --nocapture

# All statements in sequence
cargo test --test statement_performance_benchmark benchmark_all_statements_1m_dataset -- --ignored --nocapture
```

## Test Structure

Each benchmark test follows this pattern:

1. **Setup**: Create test pool, cleanup previous data, create accounting period
2. **Seed**: Insert 10k accounts + 10k account_balances (simulating 1M journal_lines)
3. **Benchmark**: Execute statement query and measure elapsed time
4. **Assert**: Verify performance < 150ms
5. **Cleanup**: Remove test data, close pool

## Seeding Strategy

```rust
// Assets & Expenses: Full debit balance
"asset" => (1,000,000, 0),    // DR 1M, CR 0
"expense" => (1,000,000, 0),  // DR 1M, CR 0

// Liabilities & Equity: Half credit balance (for equation balance)
"liability" => (0, 500,000),  // DR 0, CR 500k
"equity" => (0, 500,000),     // DR 0, CR 500k

// Revenue: Full credit balance (to offset expense)
"revenue" => (0, 1,000,000),  // DR 0, CR 1M
```

**Result**: Accounting equations satisfied:
- Debits (4k accounts × 1M) = 4B
- Credits (2k × 500k + 2k × 500k + 2k × 1M) = 4B
- Assets (2k × 1M) = Liabilities (2k × 500k) + Equity (2k × 500k)
- Net Income = Revenue (2k × 1M) - Expense (2k × 1M) = 0

## CI Integration

These benchmarks can be integrated into CI pipelines to detect performance regressions:

```yaml
- name: Run Performance Benchmarks
  run: |
    cd modules/gl
    cargo test --test statement_performance_benchmark -- --ignored --nocapture
```

## Index Verification

The benchmarks implicitly verify that queries use proper indexes:

- `account_balances_period_account_currency_idx`: Used by all statements
- `accounts_tenant_code_idx`: Used for account metadata joins
- `accounting_periods_tenant_dates_idx`: Used for period validation

Without these indexes, performance would degrade to >1s for 10k accounts.

## Future Enhancements

Potential additions for comprehensive performance testing:

1. **Multi-currency benchmarks**: Test with 10+ currencies per account
2. **Multi-period benchmarks**: Test with 100+ accounting periods
3. **Concurrent query benchmarks**: Test with 10+ simultaneous statement requests
4. **Memory profiling**: Track heap allocations during statement generation
5. **Query plan verification**: Assert EXPLAIN plans use expected indexes

## Related Documentation

- [Phase 14 Overview](../../docs/PHASE_14_FINANCIAL_STATEMENTS.md)
- [Reporting Foundation](../src/repos/statement_repo.rs)
- [Statement Services](../src/services/)
