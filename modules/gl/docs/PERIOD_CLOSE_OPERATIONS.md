# Period Close Operational Guide

**Phase 13: GL Period Close Workflow**
**Status:** Production-Ready
**Version:** 1.0.0

This document provides operational guidance for the GL period close lifecycle introduced in Phase 13.

---

## Table of Contents

1. [Overview](#overview)
2. [HTTP Endpoints](#http-endpoints)
3. [Idempotency Semantics](#idempotency-semantics)
4. [Close Snapshot & Hash](#close-snapshot--hash)
5. [Failure Modes](#failure-modes)
6. [Testing](#testing)
7. [Limitations](#limitations)

---

## Overview

The Period Close workflow provides operational controls for GL accounting periods:

- **Pre-Close Validation**: Check if a period can be closed before attempting close
- **Atomic Close**: Close a period with sealed snapshot and tamper-detection hash
- **Close Status Query**: Check the current close state of a period
- **Hard Lock Enforcement**: Block posting and reversals into closed periods

**Key Characteristics:**
- All operations are **tenant-scoped** (multi-tenancy isolation)
- Close is **atomic** (single DB transaction, all-or-nothing)
- Close is **idempotent** (safe to retry, same result)
- Close creates **immutable snapshot** (cannot be modified after close)
- Close includes **SHA-256 hash** for tamper detection

---

## HTTP Endpoints

### 1. POST /api/gl/periods/{period_id}/validate-close

**Purpose:** Pre-flight validation to check if a period can be closed.

**Request:**
```json
{
  "tenant_id": "tenant_123"
}
```

**Response (200 OK):**
```json
{
  "period_id": "550e8400-e29b-41d4-a716-446655440000",
  "tenant_id": "tenant_123",
  "can_close": true,
  "validation_report": {
    "issues": []
  },
  "validated_at": "2025-07-15T14:30:00Z"
}
```

**Response (Validation Failed - 200 OK, but can_close=false):**
```json
{
  "period_id": "550e8400-e29b-41d4-a716-446655440000",
  "tenant_id": "tenant_123",
  "can_close": false,
  "validation_report": {
    "issues": [
      {
        "severity": "ERROR",
        "code": "UNBALANCED_ENTRIES",
        "message": "Period has 3 unbalanced journal entries - debits do not equal credits",
        "metadata": {
          "unbalanced_count": 3
        }
      }
    ]
  },
  "validated_at": "2025-07-15T14:30:00Z"
}
```

**Validation Checks:**
1. **Period Exists** (tenant-scoped)
2. **Not Already Closed** (closed_at IS NULL)
3. **No Unbalanced Entries** (sum debits = sum credits for all journal entries in period)

**Error Codes:**
- `PERIOD_NOT_FOUND`: Period doesn't exist for this tenant
- `PERIOD_ALREADY_CLOSED`: Period is already closed
- `UNBALANCED_ENTRIES`: Period has journal entries where debits ≠ credits

**Usage:**
```bash
# Check if period can be closed
curl -X POST http://localhost:8090/api/gl/periods/550e8400-e29b-41d4-a716-446655440000/validate-close \
  -H "Content-Type: application/json" \
  -d '{"tenant_id": "tenant_123"}'
```

---

### 2. POST /api/gl/periods/{period_id}/close

**Purpose:** Atomically close an accounting period with sealed snapshot.

**Request:**
```json
{
  "tenant_id": "tenant_123",
  "closed_by": "admin_user",
  "close_reason": "Month-end close for January 2025"
}
```

**Response (Success - 200 OK):**
```json
{
  "period_id": "550e8400-e29b-41d4-a716-446655440000",
  "tenant_id": "tenant_123",
  "success": true,
  "close_status": {
    "state": "CLOSED",
    "closed_at": "2025-07-15T14:35:00Z",
    "closed_by": "admin_user",
    "close_reason": "Month-end close for January 2025",
    "close_hash": "a1b2c3d4e5f6...",
    "requested_at": "2025-07-15T14:35:00Z"
  },
  "validation_report": null,
  "timestamp": "2025-07-15T14:35:00Z"
}
```

**Response (Validation Failed - 200 OK, but success=false):**
```json
{
  "period_id": "550e8400-e29b-41d4-a716-446655440000",
  "tenant_id": "tenant_123",
  "success": false,
  "close_status": null,
  "validation_report": {
    "issues": [
      {
        "severity": "ERROR",
        "code": "UNBALANCED_ENTRIES",
        "message": "Period has unbalanced journal entries",
        "metadata": {"unbalanced_count": 1}
      }
    ]
  },
  "timestamp": "2025-07-15T14:35:00Z"
}
```

**Close Workflow:**
1. BEGIN transaction
2. Lock period row (`SELECT ... FOR UPDATE`)
3. Check if already closed (idempotency - if yes, return existing status)
4. Run pre-close validation (defensively re-validates)
5. Create sealed snapshot with hash
6. Update period with close fields (closed_at, closed_by, close_reason, close_hash)
7. COMMIT transaction

**Idempotency:** Safe to retry. If period is already closed, returns existing close status without mutation.

**Usage:**
```bash
# Close a period
curl -X POST http://localhost:8090/api/gl/periods/550e8400-e29b-41d4-a716-446655440000/close \
  -H "Content-Type: application/json" \
  -d '{
    "tenant_id": "tenant_123",
    "closed_by": "admin_user",
    "close_reason": "Month-end close"
  }'
```

---

### 3. GET /api/gl/periods/{period_id}/close-status

**Purpose:** Query the current close status of a period.

**Request:**
```
GET /api/gl/periods/{period_id}/close-status?tenant_id=tenant_123
```

**Response (Open Period - 200 OK):**
```json
{
  "period_id": "550e8400-e29b-41d4-a716-446655440000",
  "tenant_id": "tenant_123",
  "period_start": "2025-01-01",
  "period_end": "2025-01-31",
  "close_status": {
    "state": "OPEN"
  },
  "timestamp": "2025-07-15T14:40:00Z"
}
```

**Response (Closed Period - 200 OK):**
```json
{
  "period_id": "550e8400-e29b-41d4-a716-446655440000",
  "tenant_id": "tenant_123",
  "period_start": "2025-01-01",
  "period_end": "2025-01-31",
  "close_status": {
    "state": "CLOSED",
    "closed_at": "2025-07-15T14:35:00Z",
    "closed_by": "admin_user",
    "close_reason": "Month-end close",
    "close_hash": "a1b2c3d4e5f6...",
    "requested_at": "2025-07-15T14:35:00Z"
  },
  "timestamp": "2025-07-15T14:40:00Z"
}
```

**Response (Not Found - 404 NOT FOUND):**
```json
{
  "error": "Period 550e8400-e29b-41d4-a716-446655440000 not found for tenant tenant_123"
}
```

**Performance:** O(1) - single row lookup, no unbounded reads.

**Usage:**
```bash
# Query close status
curl http://localhost:8090/api/gl/periods/550e8400-e29b-41d4-a716-446655440000/close-status?tenant_id=tenant_123
```

---

## Idempotency Semantics

### What is Idempotent?

**Validate-Close:**
- Always safe to call multiple times
- Returns deterministic result for given DB state
- Does NOT modify any state

**Close:**
- Safe to call multiple times
- Source of truth: `accounting_periods.closed_at`
- If `closed_at IS NOT NULL` → return existing close status (no mutation)
- If `closed_at IS NULL` → execute close transaction
- Subsequent calls return the ORIGINAL close metadata (closed_by, close_reason, close_hash from first successful close)

**Close-Status:**
- Always safe to call
- Read-only query, no side effects

### What is Immutable After Close?

Once a period is closed (`closed_at` set), the following fields are **immutable**:

- `closed_at` (timestamp when closed)
- `closed_by` (user who closed)
- `close_reason` (reason provided at close time)
- `close_hash` (SHA-256 hash of sealed snapshot)
- Period summary snapshots (rows in `period_summary_snapshots` table)

**Attempts to modify these fields are BLOCKED.**

### What CAN Change After Close?

Nothing. Period close is **final and immutable**. The period cannot be reopened.

**Important:** Period reopen is **explicitly out of scope** for Phase 13. Once closed, a period remains closed permanently.

---

## Close Snapshot & Hash

### Snapshot Purpose

When a period closes, the system creates a **sealed snapshot** of period summary data:

- **Journal entry counts** (per currency)
- **Line counts** (per currency)
- **Total debits** (per currency)
- **Total credits** (per currency)
- **Account balance row count**

This snapshot is stored in `period_summary_snapshots` table with UNIQUE constraint on `(tenant_id, period_id, currency)`.

### Hash Computation

The `close_hash` is a **SHA-256 hash** computed from:

1. `tenant_id`
2. `period_id`
3. `total_journal_count` (sum across all currencies)
4. `total_debits_minor` (sum across all currencies)
5. `total_credits_minor` (sum across all currencies)
6. `balance_row_count`

**Hash Format:** Hex-encoded SHA-256 (64 characters)

**Example:**
```
a1b2c3d4e5f6789012345678901234567890123456789012345678901234abcd
```

### Tamper Detection

The hash enables **tamper detection**:

1. Store `close_hash` at close time
2. Later, recompute hash from current data
3. Compare: if hashes match → data unchanged; if mismatch → tampering detected

**Verification Function:**
```rust
verify_close_hash(pool, tenant_id, period_id, expected_hash) -> Result<(), Error>
```

Returns `Ok(())` if hash matches, `Err(HashMismatch)` if data has been altered.

---

## Failure Modes

### 1. Validation Failures

**Symptom:** `success=false` with `validation_report` containing errors

**Common Errors:**

| Error Code | Cause | Resolution |
|------------|-------|------------|
| `PERIOD_NOT_FOUND` | Period doesn't exist for tenant | Verify period_id and tenant_id are correct |
| `PERIOD_ALREADY_CLOSED` | Period is already closed | Check close-status; period cannot be reopened |
| `UNBALANCED_ENTRIES` | Journal entries have debits ≠ credits | Fix unbalanced entries before closing |

**Example Resolution (Unbalanced Entries):**
```sql
-- Find unbalanced journal entries
SELECT je.id, je.description,
       SUM(jl.debit_minor) as total_debits,
       SUM(jl.credit_minor) as total_credits
FROM journal_entries je
JOIN journal_lines jl ON jl.journal_entry_id = je.id
WHERE je.tenant_id = 'tenant_123'
  AND je.posted_at >= '2025-01-01'
  AND je.posted_at <= '2025-01-31'
GROUP BY je.id
HAVING SUM(jl.debit_minor) != SUM(jl.credit_minor);

-- Fix by adding correction lines or reversing entry
```

### 2. Database Errors

**Symptom:** HTTP 500 INTERNAL_SERVER_ERROR

**Causes:**
- Database connection pool exhausted
- Transaction timeout
- Database constraint violation

**Resolution:**
- Check database connection pool settings
- Check database logs for errors
- Verify schema integrity

### 3. Period Not Found (404)

**Symptom:** HTTP 404 NOT_FOUND for close-status endpoint

**Causes:**
- Period ID doesn't exist
- Wrong tenant_id (tenant isolation)
- Period was deleted (rare)

**Resolution:**
- Verify period_id and tenant_id
- Query `accounting_periods` table directly to confirm existence

### 4. Concurrency Conflicts

**Symptom:** None (handled transparently)

**How it Works:**
- Close uses `FOR UPDATE` row lock to prevent concurrent closes
- If two close requests arrive simultaneously:
  - First request acquires lock, proceeds with close
  - Second request waits for lock, then sees `closed_at IS NOT NULL`, returns existing status
- Result: Both requests succeed with identical close_hash

**No manual intervention needed** - idempotency handles concurrency automatically.

---

## Testing

### Service Layer Tests

**Validation Engine Tests:**
```bash
# Run pre-close validation tests
cd modules/gl
cargo test --test test_period_validation -- --test-threads=1
```

**Snapshot Sealing Tests:**
```bash
# Run snapshot creation tests
cargo test --test test_period_close_snapshot -- --test-threads=1
```

**Atomic Close Tests:**
```bash
# Run close command tests
cargo test --test test_period_close_atomic -- --test-threads=1
```

### HTTP Boundary E2E Tests

**Prerequisites:**
- GL service running on localhost:8090
- Database initialized with migrations
- NATS server running (if using NATS bus)

**Run E2E Tests:**
```bash
# Start GL service
cd modules/gl
cargo run

# In separate terminal, run E2E tests
cargo test --test boundary_e2e_http_period_close -- --test-threads=1
```

**E2E Test Coverage:**
- Validate-close success/failure
- Close success/idempotency/validation failure
- Close-status open/closed/not-found
- Performance guard (< 500ms per endpoint)

### Important Test Constraints

**MANDATORY: Use Serial Execution**

All E2E tests MUST run with `--test-threads=1` to prevent connection pool exhaustion:

```bash
# ✅ CORRECT
cargo test --test test_period_close_atomic -- --test-threads=1

# ❌ INCORRECT (will cause OOM kills)
cargo test --test test_period_close_atomic
```

**Reason:** Tests use singleton DB pool with connection caps (DB_MAX_CONNECTIONS=2). Parallel execution causes resource exhaustion.

**Environment Variables:**
```bash
export DB_MAX_CONNECTIONS=2
export DB_MIN_CONNECTIONS=0
```

**DO NOT run full `cargo test`** - this will run hundreds of tests in parallel and crash postgres containers. Always use targeted test commands.

### Performance Guards

All HTTP endpoints have performance guards:

- **Target:** < 500ms per endpoint
- **Measured in:** `test_http_period_close_performance_guard`
- **Enforcement:** CI gates (planned)

If performance degrades, check:
- Database indexes are present
- Queries are bounded (no table scans)
- Connection pool is not exhausted

---

## Limitations

### Explicitly Out of Scope

**Period Reopen:**
- **NOT SUPPORTED** in Phase 13
- Once closed, period remains closed permanently
- No "undo" or "reopen" functionality
- Future phases may add controlled reopen with audit trail

**Multi-Currency Close:**
- Snapshots are per-currency
- Close hash aggregates across all currencies
- No per-currency close control (period-level only)

**Partial Close:**
- Close is all-or-nothing for entire period
- Cannot close specific accounts or currencies within a period

**Close Reversal:**
- No mechanism to reverse a close
- If close was in error, corrective entries must be made in subsequent periods

### Known Constraints

**Tenant Isolation:**
- All operations are tenant-scoped
- Cross-tenant period access is blocked
- No shared periods between tenants

**Date Range Overlap:**
- Database constraint: `EXCLUDE (tenant_id WITH =, daterange(period_start, period_end, '[]') WITH &&)`
- Cannot create overlapping periods for same tenant
- Prevents ambiguous period assignment for journal entries

**Validation Timing:**
- Validation re-runs defensively on every close attempt
- Pre-validation does NOT guarantee close will succeed (data may change)
- Always check close response for validation errors

---

## Troubleshooting

### Common Issues

**Issue:** "Period already closed" error when trying to close
- **Cause:** Period was already closed (maybe by another user)
- **Solution:** Check close-status endpoint to see who closed it and when

**Issue:** "Unbalanced entries" error during validation
- **Cause:** Journal entries in period have debits ≠ credits
- **Solution:** Query and fix unbalanced entries (see Failure Modes section)

**Issue:** Close endpoint returns success=true but period not closed
- **Cause:** Not possible - close is atomic (transaction)
- **Check:** Query close-status to verify state

**Issue:** Different close_hash on repeated close attempts
- **Cause:** Idempotency working correctly - returns FIRST close hash
- **Verify:** Check that closed_at, closed_by, close_reason also match first close

### Debugging Commands

**Check period close state:**
```sql
SELECT id, tenant_id, period_start, period_end,
       closed_at, closed_by, close_reason, close_hash
FROM accounting_periods
WHERE tenant_id = 'tenant_123'
  AND period_start = '2025-01-01';
```

**Check period snapshots:**
```sql
SELECT period_id, currency, journal_count,
       total_debits_minor, total_credits_minor
FROM period_summary_snapshots
WHERE tenant_id = 'tenant_123'
  AND period_id = '550e8400-e29b-41d4-a716-446655440000';
```

**Find unbalanced journal entries:**
```sql
SELECT je.id, je.description,
       SUM(jl.debit_minor) as debits,
       SUM(jl.credit_minor) as credits,
       SUM(jl.debit_minor) - SUM(jl.credit_minor) as diff
FROM journal_entries je
JOIN journal_lines jl ON jl.journal_entry_id = je.id
WHERE je.tenant_id = 'tenant_123'
  AND je.posted_at::DATE >= '2025-01-01'
  AND je.posted_at::DATE <= '2025-01-31'
GROUP BY je.id
HAVING SUM(jl.debit_minor) != SUM(jl.credit_minor);
```

---

## Architecture Notes

**Transaction Boundaries:**
- Validate: Single read transaction
- Close: Single write transaction (atomic)
- Close-status: Single read (no transaction)

**Locking Strategy:**
- Close uses `SELECT ... FOR UPDATE` on period row
- Lock is surgical (exact tenant_id + period_id)
- Lock prevents concurrent close race conditions

**Snapshot Idempotency:**
- `INSERT ... ON CONFLICT (tenant_id, period_id, currency) DO NOTHING`
- Conflict is expected on retry (not an error)
- Sealed snapshots are never overwritten

**Error Handling:**
- All errors return structured JSON responses
- No panic paths in runtime code
- Database errors mapped to HTTP 500 (no internal details leaked)

---

## Support

For issues or questions:
1. Check this operational guide
2. Review error messages and validation reports
3. Consult database state with debugging SQL
4. Escalate to platform team if needed

**Version:** Phase 13, v1.0.0
**Last Updated:** 2026-02-14
