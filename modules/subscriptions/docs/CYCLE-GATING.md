# Subscription Cycle Gating (bd-184)

**Phase 15: Exactly-Once Invoice Per Subscription Cycle**

## Overview

Cycle gating ensures that each subscription cycle generates exactly one invoice, even under:
- Concurrent bill run triggers
- Event replay/retries
- Database failures mid-processing

## Architecture

### Pattern: Gate → Lock → Check → Execute → Record

```
┌─────────────┐
│   Request   │
└──────┬──────┘
       │
       ▼
┌─────────────────────────────────┐
│ 1. Generate Cycle Key           │
│    (tenant_id + subscription_id │
│     + cycle_key)                │
└──────┬──────────────────────────┘
       │
       ▼
┌─────────────────────────────────┐
│ 2. Acquire Advisory Lock        │
│    (pg_advisory_xact_lock)      │
│    - Transaction-scoped         │
│    - Auto-released on commit    │
└──────┬──────────────────────────┘
       │
       ▼
┌─────────────────────────────────┐
│ 3. Check Attempt Exists         │
│    - Query attempt ledger       │
│    - Return if duplicate        │
└──────┬──────────────────────────┘
       │
       ▼
┌─────────────────────────────────┐
│ 4. Record Attempt               │
│    - INSERT with UNIQUE         │
│    - Status: 'attempting'       │
│    - Commit (releases lock)     │
└──────┬──────────────────────────┘
       │
       ▼
┌─────────────────────────────────┐
│ 5. Execute Invoice Creation     │
│    - Call AR API (outside tx)   │
│    - Create invoice             │
│    - Finalize invoice           │
└──────┬──────────────────────────┘
       │
       ▼
┌─────────────────────────────────┐
│ 6. Record Outcome               │
│    - mark_attempt_succeeded()   │
│    - OR mark_attempt_failed()   │
└─────────────────────────────────┘
```

## Components

### 1. Migration (20260215000002_create_subscription_invoice_attempts.sql)

```sql
CREATE TABLE subscription_invoice_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id VARCHAR(255) NOT NULL,
    subscription_id UUID NOT NULL,
    cycle_key VARCHAR(20) NOT NULL,  -- YYYY-MM format
    cycle_start DATE NOT NULL,
    cycle_end DATE NOT NULL,
    status subscription_invoice_attempt_status NOT NULL,
    ar_invoice_id INTEGER,
    -- ... timestamps and metadata ...

    -- CRITICAL: Exactly-once enforcement
    CONSTRAINT unique_subscription_cycle_invoice
        UNIQUE (tenant_id, subscription_id, cycle_key)
);
```

**Guarantees:**
- UNIQUE constraint prevents duplicate invoices at database level
- Transaction-scoped advisory locks prevent concurrent attempts
- Idempotent: replay returns DuplicateCycle error

### 2. Cycle Gating Module (src/cycle_gating.rs)

**Key Functions:**

```rust
// Generate deterministic cycle key from any date in the cycle
pub fn generate_cycle_key(date: NaiveDate) -> String
// Example: 2026-02-15 → "2026-02"

// Calculate cycle boundaries (month start/end)
pub fn calculate_cycle_boundaries(date: NaiveDate) -> (NaiveDate, NaiveDate)
// Example: 2026-02-15 → (2026-02-01, 2026-02-28)

// Acquire transaction-scoped advisory lock
pub async fn acquire_cycle_lock(
    tx: &mut PgConnection,
    tenant_id: &str,
    subscription_id: Uuid,
    cycle_key: &str,
) -> Result<(), CycleGatingError>

// Check if attempt already exists (idempotency check)
pub async fn cycle_attempt_exists(
    tx: &mut PgConnection,
    tenant_id: &str,
    subscription_id: Uuid,
    cycle_key: &str,
) -> Result<bool, CycleGatingError>

// Record new attempt (status: 'attempting')
pub async fn record_cycle_attempt(...) -> Result<Uuid, CycleGatingError>

// Mark attempt as succeeded
pub async fn mark_attempt_succeeded(
    tx: &mut PgConnection,
    attempt_id: Uuid,
    ar_invoice_id: i32,
) -> Result<(), CycleGatingError>

// Mark attempt as failed
pub async fn mark_attempt_failed(
    tx: &mut PgConnection,
    attempt_id: Uuid,
    failure_code: &str,
    failure_message: &str,
) -> Result<(), CycleGatingError>
```

### 3. Gated Invoice Creation (src/gated_invoice_creation.rs)

**High-level wrapper that implements the full pattern:**

```rust
pub async fn create_gated_invoice(
    pool: &PgPool,
    tenant_id: &str,
    subscription_id: Uuid,
    ar_customer_id: i32,
    price_minor: i64,
    billing_date: NaiveDate,
    ar_base_url: &str,
) -> Result<InvoiceCreationResult, InvoiceCreationError>
```

**Usage Example:**

```rust
use subscriptions_rs::gated_invoice_creation::{create_gated_invoice, InvoiceCreationError};

match create_gated_invoice(
    &pool,
    tenant_id,
    subscription_id,
    ar_customer_id,
    price_minor,
    billing_date,
    ar_base_url,
).await {
    Ok(result) => {
        tracing::info!("Invoice created: {}", result.invoice_id);
        invoices_created += 1;
    }
    Err(InvoiceCreationError::DuplicateCycle { .. }) => {
        // Idempotent - invoice already exists for this cycle
        tracing::info!("Invoice already created (idempotent)");
    }
    Err(e) => {
        tracing::error!("Invoice creation failed: {}", e);
        failures += 1;
    }
}
```

## Integration Points

### execute_bill_run (src/routes.rs)

**Before (No Gating):**
```rust
for subscription in subscriptions {
    // Direct AR API calls
    let invoice = client.post(...).send().await?;
    let _ = client.post(&format!("{}/finalize", invoice.id)).send().await?;

    // Update next_bill_date
    sqlx::query("UPDATE subscriptions SET next_bill_date = ...").execute(&db).await?;
}
```

**After (With Gating):**
```rust
for subscription in subscriptions {
    match create_gated_invoice(
        &db,
        &subscription.tenant_id,
        subscription.id,
        ar_customer_id,
        subscription.price_minor,
        execution_date,
        &ar_base_url,
    ).await {
        Ok(result) => {
            invoices_created += 1;

            // Update next_bill_date
            let new_next_bill_date = calculate_next_bill_date(...);
            sqlx::query("UPDATE subscriptions SET next_bill_date = $1 WHERE id = $2")
                .bind(new_next_bill_date)
                .bind(subscription.id)
                .execute(&db)
                .await?;
        }
        Err(InvoiceCreationError::DuplicateCycle { .. }) => {
            // Idempotent - already processed
            tracing::info!("Invoice already created for cycle");
        }
        Err(e) => {
            tracing::error!("Invoice creation failed: {}", e);
            failures += 1;
        }
    }
}
```

## Guarantees

### Exactly-Once (Database Level)

**UNIQUE Constraint:**
```sql
CONSTRAINT unique_subscription_cycle_invoice
    UNIQUE (tenant_id, subscription_id, cycle_key)
```

Any attempt to insert duplicate (tenant_id, subscription_id, cycle_key) will fail with:
- PostgreSQL error: `duplicate key value violates unique constraint`
- Mapped to: `CycleGatingError::DuplicateCycle`

### Concurrent Safety (Advisory Locks)

**Advisory Lock Key Generation:**
```rust
fn generate_advisory_lock_key(
    tenant_id: &str,
    subscription_id: Uuid,
    cycle_key: &str,
) -> i64 {
    // Hash (tenant_id, subscription_id, cycle_key) → i64
    // Deterministic: same inputs → same lock key
    // Unique: different subscriptions/cycles → different locks
}
```

**Lock Scope:**
- `pg_advisory_xact_lock`: Transaction-scoped
- Automatically released on COMMIT or ROLLBACK
- No explicit unlock needed

**Concurrency Scenario:**
```
Time  │ Request A                  │ Request B
──────┼────────────────────────────┼────────────────────────────
t0    │ BEGIN                      │ BEGIN
t1    │ pg_advisory_xact_lock(123) │ pg_advisory_xact_lock(123) [BLOCKS]
t2    │ Check: no attempt exists   │ [WAITING]
t3    │ INSERT attempt             │ [WAITING]
t4    │ COMMIT (releases lock)     │ [WAITING]
t5    │                            │ [UNBLOCKED]
t6    │                            │ Check: attempt EXISTS
t7    │                            │ ROLLBACK (duplicate)
```

### Replay Safety (Idempotency)

**Idempotency Check:**
```rust
if cycle_attempt_exists(&mut tx, tenant_id, subscription_id, &cycle_key).await? {
    tracing::info!("Invoice already created for this cycle (idempotent)");
    tx.rollback().await?;
    return Err(InvoiceCreationError::DuplicateCycle { ... });
}
```

**Replay Scenarios:**
1. **Duplicate bill_run trigger:** Returns `DuplicateCycle` immediately
2. **Event replay:** UNIQUE constraint prevents duplicate INSERT
3. **Partial failure recovery:** Existing attempt found, no duplicate created

## Testing

### Unit Tests (src/cycle_gating.rs)

**Coverage:**
- Cycle key generation determinism
- Cycle boundary calculation (including leap years)
- Advisory lock key determinism and uniqueness
- 11 tests, all passing ✅

### Integration Tests (tests/cycle_gating_integration_test.rs)

**Coverage:**
- Attempt ledger operations (record, succeed, fail)
- UNIQUE constraint enforcement (duplicate prevention)
- Advisory lock acquisition and release
- Different cycles don't block each other
- 11 tests, all compiled ✅

**Run Tests:**
```bash
# Unit tests
cargo test --lib cycle_gating

# Integration tests (requires database)
export DATABASE_URL="postgres://postgres:postgres@localhost:5433/subscriptions_test"
cargo test --test cycle_gating_integration_test
```

## Performance Considerations

### Advisory Lock Scope

**Lock Duration:**
```
┌─────────────────────────────────────────────┐
│ Transaction Scope (< 50ms typically)        │
├─────────────────────────────────────────────┤
│ BEGIN                                       │
│ pg_advisory_xact_lock (instant)             │
│ Check attempt exists (< 1ms)                │
│ INSERT attempt (< 5ms)                      │
│ COMMIT (< 5ms)                              │
└─────────────────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────────┐
│ AR API Calls (OUTSIDE lock, 100-500ms)     │
├─────────────────────────────────────────────┤
│ POST /api/ar/invoices (200ms)               │
│ POST /api/ar/invoices/{id}/finalize (100ms)│
└─────────────────────────────────────────────┘
```

**Key Optimization:** Advisory lock is released BEFORE expensive AR API calls.
This minimizes lock contention and allows concurrent processing of different cycles.

### Database Indexes

```sql
-- Query: Check if attempt exists
CREATE INDEX subscription_invoice_attempts_tenant_subscription
    ON subscription_invoice_attempts(tenant_id, subscription_id);

-- Query: Filter by status
CREATE INDEX subscription_invoice_attempts_status
    ON subscription_invoice_attempts(status);

-- Query: Reporting by cycle
CREATE INDEX subscription_invoice_attempts_cycle
    ON subscription_invoice_attempts(cycle_start, cycle_end);
```

## Monitoring

### Key Metrics

```sql
-- Successful invoice creations
SELECT COUNT(*) FROM subscription_invoice_attempts
WHERE status = 'succeeded'
  AND attempted_at > NOW() - INTERVAL '1 hour';

-- Failed attempts (investigate)
SELECT failure_code, COUNT(*) FROM subscription_invoice_attempts
WHERE status = 'failed_final'
  AND attempted_at > NOW() - INTERVAL '1 hour'
GROUP BY failure_code;

-- Duplicate attempts (idempotency working)
SELECT COUNT(*) FROM subscription_invoice_attempts
WHERE cycle_key IN (
    SELECT cycle_key FROM subscription_invoice_attempts
    GROUP BY tenant_id, subscription_id, cycle_key
    HAVING COUNT(*) > 1
);
```

### Logging

```rust
tracing::info!(
    tenant_id = tenant_id,
    subscription_id = %subscription_id,
    cycle_key = &cycle_key,
    attempt_id = %attempt_id,
    "Gated invoice creation succeeded"
);
```

## Troubleshooting

### Issue: Duplicate Invoices

**Symptom:** Multiple invoices for same subscription cycle in AR

**Diagnosis:**
```sql
-- Check for duplicate attempts
SELECT tenant_id, subscription_id, cycle_key, COUNT(*) as attempts
FROM subscription_invoice_attempts
WHERE status = 'succeeded'
GROUP BY tenant_id, subscription_id, cycle_key
HAVING COUNT(*) > 1;
```

**Root Cause:** UNIQUE constraint not enforced or migration not applied

**Resolution:**
1. Verify migration applied: `SELECT * FROM subscription_invoice_attempts LIMIT 1;`
2. Check constraint exists: `\d subscription_invoice_attempts` (psql)
3. Re-run migration if needed

### Issue: Advisory Lock Deadlock

**Symptom:** Transactions timing out or deadlocking

**Diagnosis:**
```sql
-- Check active advisory locks
SELECT * FROM pg_locks WHERE locktype = 'advisory';
```

**Root Cause:** Lock key collision (hash collision) or improper lock ordering

**Resolution:**
1. Advisory lock keys are i64 (64-bit) with minimal collision probability
2. Ensure consistent lock ordering across transactions
3. Transaction-scoped locks auto-release on commit/rollback

### Issue: Stuck 'attempting' Status

**Symptom:** Attempts remain in 'attempting' status indefinitely

**Diagnosis:**
```sql
-- Find old attempting records
SELECT * FROM subscription_invoice_attempts
WHERE status = 'attempting'
  AND attempted_at < NOW() - INTERVAL '1 hour';
```

**Root Cause:** Process crashed/killed after INSERT but before marking succeeded/failed

**Resolution:**
1. Manual investigation: Check if AR invoice was actually created
2. If created: Update status to 'succeeded' with ar_invoice_id
3. If not created: Update status to 'failed_final' with failure reason
4. Consider adding cleanup job for stale 'attempting' records

## References

- **Phase 15 ADR:** docs/architecture/decisions/ADR-015-phase15-lifecycle-enforcement-posture.md
- **bd-1p2 Idempotency Keys:** modules/ar/src/idempotency_keys.rs
- **bd-7gl Attempt Ledgers:** modules/ar/db/migrations/20260215000001_create_invoice_attempts.sql
- **PostgreSQL Advisory Locks:** https://www.postgresql.org/docs/current/explicit-locking.html#ADVISORY-LOCKS
