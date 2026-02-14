# bd-qmj: Hard Lock Semantics Enforcement - Implementation Plan

## Status
- **Bead:** bd-qmj
- **Assignee:** GoldValley
- **Status:** Waiting for dependencies (bd-3rx, bd-1zp)

## Dependencies
1. **bd-3rx** (IN_PROGRESS, EmeraldBear): Schema Extensions
   - Adds: `closed_at`, `closed_by`, `close_reason`, `close_hash` to `accounting_periods`
   - Migration file: Expected `20260214000001_add_period_close_fields.sql`

2. **bd-1zp** (OPEN, EmeraldBear): Atomic Close Command
   - Implements close operation that sets `closed_at`
   - Idempotency via `closed_at.is_some()`

## Requirements

### 1. Posting Enforcement Update
**Current State:**
- `modules/gl/src/services/journal_service.rs:96`
- Checks `period.is_closed` boolean

**Required Change:**
- Update to check `period.closed_at.is_some()` instead
- Error message should remain stable (use existing `PeriodError::PeriodClosed`)

**Code Location:**
```rust
// File: modules/gl/src/services/journal_service.rs
// Line: ~96

// BEFORE (Phase 10):
if period.is_closed {
    return Err(JournalError::Period(period_repo::PeriodError::PeriodClosed {
        tenant_id: tenant_id.to_string(),
        date: posting_date,
        period_id: period.id,
    }));
}

// AFTER (Phase 13):
if period.closed_at.is_some() {
    return Err(JournalError::Period(period_repo::PeriodError::PeriodClosed {
        tenant_id: tenant_id.to_string(),
        date: posting_date,
        period_id: period.id,
    }));
}
```

### 2. Reversal Enforcement Update (NEW REQUIREMENT)
**Current State:**
- `modules/gl/src/services/reversal_service.rs:92`
- Only checks if reversal date's period is closed
- Does NOT check original entry's period

**Required Change:**
- Keep existing check for reversal period
- ADD new check: original entry's period must NOT be closed
- If original period is closed → reject reversal with stable error

**Implementation Steps:**
1. After loading original entry (line 64-66), get the original entry's period
2. Check if original period is closed (`original_period.closed_at.is_some()`)
3. If closed, return new error variant: `ReversalError::OriginalPeriodClosed`

**Code Location:**
```rust
// File: modules/gl/src/services/reversal_service.rs
// After line 75 (before transaction start)

// Get the original entry's period
let original_period = period_repo::find_by_date(
    pool,
    &original_entry.tenant_id,
    original_entry.entry_date.date()
)
.await?
.ok_or_else(|| {
    period_repo::PeriodError::NoPeriodForDate {
        tenant_id: original_entry.tenant_id.clone(),
        date: original_entry.entry_date.date(),
    }
})?;

// Check if original entry's period is closed
if original_period.closed_at.is_some() {
    return Err(ReversalError::OriginalPeriodClosed {
        original_entry_id,
        period_id: original_period.id,
        closed_at: original_period.closed_at.unwrap(),
    });
}
```

### 3. Error Type Updates

**Add new error variant to ReversalError:**
```rust
// File: modules/gl/src/services/reversal_service.rs
// In ReversalError enum

#[error("Cannot reverse entry {original_entry_id} - original period {period_id} was closed at {closed_at}")]
OriginalPeriodClosed {
    original_entry_id: Uuid,
    period_id: Uuid,
    closed_at: chrono::DateTime<chrono::Utc>,
},
```

### 4. Period Repo Updates

**Update Period struct:**
```rust
// File: modules/gl/src/repos/period_repo.rs
// In Period struct

pub struct Period {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub is_closed: bool,  // Deprecated but kept for compatibility
    pub created_at: DateTime<Utc>,
    // Phase 13 additions:
    pub closed_at: Option<DateTime<Utc>>,
    pub closed_by: Option<String>,
    pub close_reason: Option<String>,
    pub close_hash: Option<String>,
}
```

**Update validation functions:**
```rust
// File: modules/gl/src/repos/period_repo.rs
// Lines ~122 and ~145

// Change from:
Some(p) if p.is_closed => Err(...)

// To:
Some(p) if p.closed_at.is_some() => Err(...)
```

## Testing Strategy

### Integration Tests (Service Layer)

**Test File:** `modules/gl/tests/period_close_enforcement_test.rs` (NEW)

**Test Cases:**

1. **test_posting_blocked_when_period_closed**
   - Setup: Create period, close it (set closed_at)
   - Action: Attempt to post to that period
   - Assert: Posting rejected with PeriodClosed error

2. **test_reversal_blocked_when_original_period_closed**
   - Setup: Create entry in period A, close period A
   - Action: Attempt to reverse the entry (reversal in period B)
   - Assert: Reversal rejected with OriginalPeriodClosed error

3. **test_reversal_blocked_when_reversal_period_closed**
   - Setup: Create entry in period A, create period B, close B
   - Action: Attempt to reverse entry (reversal would go to period B)
   - Assert: Reversal rejected with PeriodClosed error

4. **test_reversal_succeeds_when_both_periods_open**
   - Setup: Create entry in period A (open), reversal to period B (open)
   - Action: Reverse the entry
   - Assert: Reversal succeeds

5. **test_closed_at_semantics_override_is_closed_boolean**
   - Setup: Period with is_closed=false but closed_at=Some(timestamp)
   - Action: Attempt posting
   - Assert: Blocked (closed_at takes precedence)

### Error Message Validation

**Test stable error codes:**
- `PeriodError::PeriodClosed` message format unchanged
- `ReversalError::OriginalPeriodClosed` message clear and actionable

## Files to Modify

1. `modules/gl/src/repos/period_repo.rs`
   - Update Period struct with new fields
   - Update validation logic (is_closed → closed_at)

2. `modules/gl/src/services/journal_service.rs`
   - Update posting enforcement (line ~96)

3. `modules/gl/src/services/reversal_service.rs`
   - Add original period closed check
   - Add OriginalPeriodClosed error variant

4. `modules/gl/tests/period_close_enforcement_test.rs` (NEW)
   - Integration tests for enforcement

## Acceptance Criteria Checklist

- [ ] Posting attempts into closed periods fail with PeriodClosed error
- [ ] Reversal attempts for entries whose original period is closed fail with OriginalPeriodClosed error
- [ ] Both enforcements use `closed_at.is_some()` semantics (not is_closed boolean)
- [ ] Error messages are stable and actionable
- [ ] Integration tests pass (4+ test cases covering all paths)
- [ ] No unwrap/expect in runtime paths
- [ ] Enforcement cannot be bypassed through alternate routes

## Implementation Order

1. Wait for bd-3rx migration to land (adds closed_at field)
2. Update Period struct in period_repo.rs
3. Update posting enforcement in journal_service.rs
4. Update reversal enforcement in reversal_service.rs (add original period check)
5. Write integration tests
6. Run tests: `cargo test --test period_close_enforcement_test -- --test-threads=1`
7. Verify error messages are stable
8. Commit with `[bd-qmj]` prefix

## Migration Dependency

**Expected migration from bd-3rx:**
```sql
-- File: modules/gl/db/migrations/20260214000001_add_period_close_fields.sql

ALTER TABLE accounting_periods
ADD COLUMN closed_at TIMESTAMP WITH TIME ZONE,
ADD COLUMN closed_by TEXT,
ADD COLUMN close_reason TEXT,
ADD COLUMN close_hash TEXT;

-- Index for closed period lookups
CREATE INDEX idx_accounting_periods_closed_at
ON accounting_periods(tenant_id, closed_at)
WHERE closed_at IS NOT NULL;
```

## Notes

- Keep `is_closed` boolean for backward compatibility (read-only, deprecated)
- `closed_at` is source of truth for Phase 13+
- Pre-validation MUST re-run on every close call (per ChatGPT guardrail)
- Idempotency via `closed_at` field, not event_id (per ChatGPT guardrail)
