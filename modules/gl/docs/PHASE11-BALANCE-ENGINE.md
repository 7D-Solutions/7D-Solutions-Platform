# Phase 11: Balance Engine Specification

## Purpose

Define the Balance Engine as a **deterministic, audit-safe read model** built from journal entries (the source of truth). This specification locks the contract for what a "balance" means, how balances are computed, and how they are maintained, ensuring implementation remains consistent with Phase 10 governance and audit requirements.

---

## Overview

The Balance Engine materializes account balances from journal entries to enable:
- Fast balance queries without scanning journal history
- Trial balance generation
- Period-over-period reporting
- Foundation for future AP/AR control and analytics

**Key Principles:**
- Journal entries are the **source of truth**
- Balances are **derived projections** (read model)
- All Phase 10 governance preserved (COA validation, period enforcement, idempotency)
- Multi-currency support (ISO 4217)
- Deterministic: same journal = same balances
- Audit-safe: no silent corrections, explicit error handling

---

## Balance Grain Definition

A balance record represents the **cumulative financial position** for a unique combination of dimensions.

### Grain (Unique Key)
```
UNIQUE (tenant_id, period_id, account_code, currency)
```

**Dimensions:**
- `tenant_id` (TEXT) - Tenant isolation
- `period_id` (UUID) - Accounting period reference (FK to accounting_periods.id)
- `account_code` (TEXT) - Chart of Accounts code (e.g., "1000", "4000")
- `currency` (TEXT) - ISO 4217 currency code (e.g., "USD", "EUR", "GBP")

**Why this grain?**
1. **tenant_id** - Multi-tenant isolation (Phase 10)
2. **period_id** - Period-aware balances enable period close and comparative reporting
3. **account_code** - Balance per account (COA-aware, Phase 10)
4. **currency** - Multi-currency support without lossy conversion

---

## Schema Definition

### Table: `balance_snapshots`

```sql
CREATE TABLE balance_snapshots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    period_id UUID NOT NULL REFERENCES accounting_periods(id),
    account_code TEXT NOT NULL,
    currency TEXT NOT NULL,

    -- Cumulative amounts (in minor units, e.g., cents)
    debit_total_minor BIGINT NOT NULL DEFAULT 0 CHECK (debit_total_minor >= 0),
    credit_total_minor BIGINT NOT NULL DEFAULT 0 CHECK (credit_total_minor >= 0),

    -- Net balance (signed, in minor units)
    -- Positive = net debit position, Negative = net credit position
    net_balance_minor BIGINT NOT NULL DEFAULT 0,

    -- Metadata
    last_journal_entry_id UUID, -- Last entry that updated this balance
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Unique constraint on grain
    CONSTRAINT unique_balance_grain UNIQUE (tenant_id, period_id, account_code, currency)
);
```

### Indexes

```sql
-- Primary lookup: tenant + period + account
CREATE INDEX idx_balance_snapshots_tenant_period
    ON balance_snapshots(tenant_id, period_id);

-- Tenant + period lookup (for trial balance)
CREATE INDEX idx_balance_snapshots_tenant_period_full
    ON balance_snapshots(tenant_id, period_id, account_code, currency);

-- Account-centric queries
CREATE INDEX idx_balance_snapshots_account
    ON balance_snapshots(tenant_id, account_code);

-- Period FK integrity
CREATE INDEX idx_balance_snapshots_period_id
    ON balance_snapshots(period_id);

-- Updated_at for incremental processing
CREATE INDEX idx_balance_snapshots_updated_at
    ON balance_snapshots(updated_at);
```

---

## Balance Calculation Rules

### Net Balance Semantics

```
net_balance_minor = debit_total_minor - credit_total_minor
```

**Interpretation:**
- `net_balance_minor > 0` → **Net debit position** (typical for Assets, Expenses)
- `net_balance_minor < 0` → **Net credit position** (typical for Liabilities, Equity, Revenue)
- `net_balance_minor = 0` → **Balanced** (debits equal credits)

### Normal Balance Alignment (Phase 10 COA Integration)

**Chart of Accounts defines expected balance direction:**
- Assets: `normal_balance = 'debit'` → expect `net_balance_minor > 0`
- Liabilities: `normal_balance = 'credit'` → expect `net_balance_minor < 0`
- Equity: `normal_balance = 'credit'` → expect `net_balance_minor < 0`
- Revenue: `normal_balance = 'credit'` → expect `net_balance_minor < 0`
- Expenses: `normal_balance = 'debit'` → expect `net_balance_minor > 0`

**Phase 11 Scope:**
- Balance Engine computes net balances **without validation against normal_balance**
- Reporting/analytics layers should flag abnormal balances (e.g., negative asset balance)
- Future Phase: Add balance validation/alerts as separate concern

---

## Transaction Execution Order

Balance updates **MUST** occur within the **same transaction** as journal entry creation to maintain consistency.

### Posting Transaction Sequence

```
1. BEGIN TRANSACTION
2. Check idempotency (processed_events)
3. Validate COA (accounts exist and active) [Phase 10]
4. Validate posting period (not closed) [Phase 10]
5. Insert journal_entry header
6. Insert journal_lines
7. → COMPUTE BALANCE DELTAS (New in Phase 11)
8. → UPSERT BALANCE SNAPSHOTS (New in Phase 11)
9. Mark event as processed
10. COMMIT TRANSACTION
```

### Balance Delta Computation

For each journal line in the entry:
```rust
// Pseudocode
for line in journal_lines {
    let delta = BalanceDelta {
        tenant_id: entry.tenant_id,
        period_id: period_from_posted_at(entry.posted_at), // Lookup period
        account_code: line.account_ref,
        currency: entry.currency,
        debit_delta: line.debit_minor,
        credit_delta: line.credit_minor,
    };
    balance_deltas.push(delta);
}
```

### Balance Upsert Logic

```sql
INSERT INTO balance_snapshots (
    tenant_id, period_id, account_code, currency,
    debit_total_minor, credit_total_minor, net_balance_minor,
    last_journal_entry_id, updated_at
)
VALUES ($1, $2, $3, $4, $5, $6, $5 - $6, $7, NOW())
ON CONFLICT (tenant_id, period_id, account_code, currency)
DO UPDATE SET
    debit_total_minor = balance_snapshots.debit_total_minor + EXCLUDED.debit_total_minor,
    credit_total_minor = balance_snapshots.credit_total_minor + EXCLUDED.credit_total_minor,
    net_balance_minor = (balance_snapshots.debit_total_minor + EXCLUDED.debit_total_minor)
                      - (balance_snapshots.credit_total_minor + EXCLUDED.credit_total_minor),
    last_journal_entry_id = EXCLUDED.last_journal_entry_id,
    updated_at = NOW();
```

**Key Properties:**
- **Idempotent at event level** - same `source_event_id` processed once
- **Atomic** - balance updates in same TX as journal
- **Cumulative** - deltas add to existing totals
- **Period-aware** - balance per period (not just running total)

---

## Reversal Handling (Phase 10 Integration)

When a journal entry is reversed (Phase 10 capability):

### Reversal Transaction
```
1. Original entry: Debit A $100, Credit B $100 (entry_id = E1)
2. Reversal entry: Debit B $100, Credit A $100 (entry_id = E2, reverses_entry_id = E1)
```

### Balance Impact
```
Account A:
- After E1: debit +100, credit 0, net = +100
- After E2: debit +100, credit +100, net = 0 ✓

Account B:
- After E1: debit 0, credit +100, net = -100
- After E2: debit +100, credit +100, net = 0 ✓
```

**Result:** Reversals naturally zero out balances through standard balance update logic.

**No special handling required** - reversal entries are just normal journal entries with inverted lines.

---

## Multi-Currency Handling

### Currency Isolation

**Each balance record is currency-specific:**
- Account "1000" with USD → separate balance from Account "1000" with EUR
- **No automatic conversion** - balances stored in native currency
- **Grain enforcement** - UNIQUE constraint prevents currency mixing

### ISO 4217 Compliance

**Requirements:**
- Currency codes MUST be valid ISO 4217 (uppercase, 3 letters)
- Validation at posting time (inherited from Phase 9/10 posting validation)
- Examples: "USD", "EUR", "GBP", "JPY", "CAD"

**Future Extension (Not Phase 11):**
- Multi-currency reporting requires exchange rate table
- Conversion happens at query/reporting time, not at balance persistence

---

## Invariants (Must Always Hold)

### Database-Level Invariants

1. **Non-negative totals**
   ```sql
   CHECK (debit_total_minor >= 0)
   CHECK (credit_total_minor >= 0)
   ```

2. **Grain uniqueness**
   ```sql
   UNIQUE (tenant_id, period_id, account_code, currency)
   ```

3. **Net balance consistency**
   ```
   net_balance_minor = debit_total_minor - credit_total_minor
   ```
   (Maintained by upsert logic, not DB constraint)

4. **Period FK integrity**
   ```sql
   period_id REFERENCES accounting_periods(id)
   ```

### Application-Level Invariants

1. **Tenant isolation** - Never aggregate balances across tenants
2. **Period boundary** - Balance for period P only includes journal entries with `posted_at` in period P date range
3. **Account existence** - `account_code` MUST exist in Chart of Accounts at time of posting (inherited from Phase 10)
4. **Currency consistency** - All lines in a journal entry have same currency (inherited from Phase 9 contract)

### Audit Invariants

1. **Reproducibility** - Reprocessing same journals produces same balances
2. **Traceability** - `last_journal_entry_id` links balance to source
3. **Immutability of source** - Journal entries never updated/deleted (append-only)
4. **Balance-to-journal reconciliation** - Sum of journal lines MUST equal balance totals

---

## Failure Modes and Error Handling

### Synchronous Failures (Within Posting TX)

| Failure | Cause | Behavior | Retry? |
|---------|-------|----------|--------|
| Period not found | No accounting period defined for `posted_at` date | Rollback TX, send to DLQ | ❌ No (config error) |
| Period closed | Posting to closed period | Rollback TX, send to DLQ | ❌ No (policy violation) |
| Account not found | `account_code` not in COA | Rollback TX, send to DLQ | ❌ No (data error) |
| Account inactive | `is_active = false` in COA | Rollback TX, send to DLQ | ❌ No (policy violation) |
| Database deadlock | Concurrent balance updates | Rollback TX, retry with backoff | ✅ Yes (transient) |
| DB connection lost | Network/pool exhaustion | Rollback TX, retry with backoff | ✅ Yes (transient) |
| Constraint violation | Unexpected data issue | Rollback TX, send to DLQ | ❌ No (investigate) |

### Retry Strategy

**Retry with exponential backoff** (inherited from Phase 7/8):
- Max attempts: 3
- Base delay: 100ms
- Backoff factor: 2x
- Only retry **transient errors** (DB connection, deadlock, timeouts)

**Non-retriable errors → DLQ immediately:**
- Validation failures (bad account, closed period)
- Constraint violations
- Data integrity issues

### DLQ (Dead Letter Queue)

**Table:** `failed_events` (existing from Phase 9)

**DLQ Entry Contents:**
- `event_id` - Original event ID
- `subject` - "gl.events.posting.requested" or "gl.events.reversal.requested"
- `payload` - Full event payload
- `error_message` - Detailed error with failure reason
- `retry_count` - Number of retry attempts before DLQ
- `failed_at` - Timestamp of final failure

**DLQ Monitoring:**
- Non-empty DLQ requires investigation
- Check for config issues (missing periods, inactive accounts)
- Review payload for data quality issues

---

## Phase 10 Governance Preservation

Balance Engine implementation **MUST** preserve all Phase 10 governance:

### 1. Chart of Accounts (COA) Validation
- ✅ **Inherited** - Posting validation ensures only valid, active accounts create journal entries
- ✅ Balance Engine processes only valid journal entries
- ✅ `account_code` in balances always references valid COA entry

### 2. Accounting Period Enforcement
- ✅ **Inherited** - Cannot post to closed periods
- ✅ Balance Engine computes balances only for open period postings
- ✅ Period close workflow (future) will freeze balances for closed periods

### 3. Idempotency
- ✅ **Inherited** - `source_event_id` deduplication at posting level
- ✅ Same event processed once → same journal → same balance update once
- ✅ Balance upserts are cumulative but event-level idempotent

### 4. Retry Discipline
- ✅ **Inherited** - Retry with backoff for transient errors only
- ✅ Non-retriable errors → DLQ (no silent correction)
- ✅ Balance updates use same retry strategy as posting

### 5. Reversal Capability
- ✅ **Inherited** - Reversals create journal entries with inverted lines
- ✅ Balance Engine treats reversals as normal postings
- ✅ Reversals naturally zero out balances (no special logic)

### 6. Audit Trail
- ✅ **Inherited** - Journal entries immutable (append-only)
- ✅ `last_journal_entry_id` in balances provides lineage
- ✅ Full audit trail from balance → journal → source event

---

## Period Handling Deep Dive

### Period Lookup Strategy

**At posting time:**
```rust
// Pseudocode
let posted_at: DateTime<Utc> = entry.posted_at;
let posting_date: NaiveDate = posted_at.date_naive();

// Lookup period containing this date (inherited Phase 10 logic)
let period = period_repo::find_by_date_tx(&mut tx, tenant_id, posting_date).await?;

match period {
    None => return Err(PeriodError::NoPeriodForDate { tenant_id, date: posting_date }),
    Some(p) if p.is_closed => return Err(PeriodError::PeriodClosed { period_id: p.id }),
    Some(p) => {
        // Use period.id for balance grain
        let period_id = p.id;
        // ... proceed with balance update
    }
}
```

### Period Boundary Rules

**Critical:** Balance for period P includes **only** journal entries where:
```
period.period_start <= posted_at.date() <= period.period_end
```

**Example:**
```
Period: Jan 2024 (2024-01-01 to 2024-01-31)
Journal Entry: posted_at = 2024-01-15T10:30:00Z
Result: Balance updated for Jan 2024 period_id
```

**Cross-period posting:**
- Each entry belongs to exactly one period (by `posted_at` date)
- Multi-period reporting = aggregate balances across multiple period_id values

### Period Not Found → DLQ

**If no period defined for posting date:**
- Posting fails with `PeriodError::NoPeriodForDate`
- Transaction rolls back
- Event sent to DLQ with clear message
- **Action required:** Define accounting period before posting can succeed

---

## Implementation Phases (Phase 11 Breakdown)

Phase 11 can be decomposed into incremental beads:

### Phase 11A: Schema + Migration
- Create `balance_snapshots` table with grain + indexes
- Add migration to `modules/gl/db/migrations/`
- Verify migration applies cleanly in Docker

### Phase 11B: Balance Repository
- Create `BalanceSnapshot` model struct
- Implement `upsert_balance()` with conflict handling
- Implement `find_balance()` for lookups
- Unit tests for upsert logic (including conflict resolution)

### Phase 11C: Balance Service
- Create `balance_service.rs` with delta computation
- Implement `compute_deltas_from_journal()` function
- Implement `update_balances_from_entry()` transactional logic
- Unit tests for delta computation

### Phase 11D: Posting Integration
- Wire balance service into `journal_service::process_gl_posting_request()`
- Add balance update step after journal line insert
- Ensure balance update in same TX as journal persistence
- Integration tests (journal + balance atomicity)

### Phase 11E: Reversal Integration
- Wire balance service into reversal consumer
- Verify reversals correctly update balances (zero-out test)
- E2E test: post → reverse → assert balances = 0

### Phase 11F: Trial Balance Primitive
- Create `get_trial_balance(tenant_id, period_id)` endpoint
- Return all balances for tenant+period
- Include account metadata (name, type, normal_balance) from COA
- E2E test: post entries → query trial balance → validate

### Phase 11G: Observability
- Add balance update spans to tracing
- Emit `gl.events.balance.updated` event (optional, for downstream consumers)
- Add metrics for balance update latency

### Phase 11H: Balance Reconciliation Tool
- CLI/admin tool: recompute balances from journal history
- Use case: repair drift, migrate legacy data
- Idempotent: safe to re-run
- Validation: compare recomputed vs stored balances

---

## Testing Strategy

### Unit Tests
- ✅ Balance delta computation from journal lines
- ✅ Upsert conflict resolution (insert vs update)
- ✅ Net balance calculation correctness
- ✅ Multi-currency isolation (separate balances per currency)

### Integration Tests
- ✅ Transaction atomicity (journal + balance in same TX)
- ✅ Rollback on balance update failure
- ✅ Concurrent posting to same account (deadlock handling)
- ✅ Period boundary (balance in correct period)

### E2E Tests (Docker Compose)
- ✅ Post journal entry → verify balance created
- ✅ Post multiple entries → verify cumulative balance
- ✅ Reverse entry → verify balance zeroed
- ✅ Multi-currency: post USD and EUR → verify separate balances
- ✅ Closed period posting → verify rejection + DLQ
- ✅ Invalid account → verify rejection + DLQ
- ✅ Trial balance query → verify all balances returned

### Audit Tests
- ✅ Reconciliation: sum(journal_lines) = balance_totals
- ✅ Reproducibility: delete balances, reprocess journals → same result
- ✅ Tenant isolation: balance query filters by tenant_id

---

## Performance Considerations

### Write Performance
- **Upsert overhead:** `ON CONFLICT DO UPDATE` slightly slower than insert-only
- **Mitigation:** Proper indexing on grain columns (already specified)
- **Concurrent writes:** Postgres row-level locking (balance grain = lock key)
- **Expected TPS:** 100-500 postings/sec per tenant (Phase 11 target)

### Read Performance
- **Trial balance query:** Single index scan on `(tenant_id, period_id)`
- **Account balance history:** Index on `(tenant_id, account_code)`
- **Expected latency:** <10ms for trial balance (1000 accounts)

### Scalability
- **Tenant sharding:** Balance table partitionable by `tenant_id` (future)
- **Period archival:** Move old period balances to cold storage (future)
- **Phase 11 scope:** Single-node Postgres sufficient for MVP

---

## Open Questions (Must Resolve Before Implementation)

### ✅ RESOLVED: Period Assignment
**Q:** How is `period_id` determined for a balance?
**A:** Lookup period by `posted_at.date()` using `period_repo::find_by_date_tx()` (inherited from Phase 10)

### ✅ RESOLVED: Reversal Treatment
**Q:** Do reversals require special balance logic?
**A:** No - reversals are journal entries with inverted lines, processed identically to normal postings

### ✅ RESOLVED: Multi-currency Conversion
**Q:** Should balances be converted to reporting currency?
**A:** No - Phase 11 stores native currency balances only. Conversion is reporting-layer concern (future phase)

### ✅ RESOLVED: Sync vs Async
**Q:** Should balance updates be in-TX (sync) or projector (async)?
**A:** In-TX (sync) - balance consistency with journal is critical for audit integrity

### ✅ RESOLVED: Balance Grain Period Scope
**Q:** Are balances cumulative across periods or per-period only?
**A:** Per-period only - opening balance for period P+1 computed from closing balance of period P (future phase)

---

## Success Criteria

Phase 11 Balance Engine is **complete** when:

1. ✅ `balance_snapshots` table exists with specified schema and indexes
2. ✅ Posting transaction atomically creates journal + updates balances
3. ✅ Reversals correctly zero out balances
4. ✅ Multi-currency balances isolated (separate records per currency)
5. ✅ Trial balance query returns correct balances for tenant+period
6. ✅ E2E tests pass: posting, reversal, trial balance, error cases
7. ✅ DLQ captures period/account validation failures
8. ✅ Reconciliation: sum(journal_lines) = balance_totals for all tenants
9. ✅ No open questions remain in this spec
10. ✅ CI gates pass: tests, contract validation, no panics

---

## Summary

The Phase 11 Balance Engine is a **deterministic, audit-safe materialized view** of journal entries that:
- **Preserves** all Phase 10 governance (COA, periods, idempotency, reversals)
- **Enforces** multi-currency isolation (ISO 4217)
- **Maintains** transactional consistency (balance + journal atomic)
- **Handles** failures gracefully (retry transient, DLQ non-retriable)
- **Enables** fast reporting (trial balance, period-over-period)

After Phase 11, the GL module provides:
- ✅ Journal source of truth (Phase 9)
- ✅ Governance layer (Phase 10)
- ✅ **Balance engine (Phase 11)** ← You are here
- → Next: Period close workflow, AP/AR integration, financial reporting

This spec is **locked**. Implementation must follow this design. Any deviations require spec update and approval.
