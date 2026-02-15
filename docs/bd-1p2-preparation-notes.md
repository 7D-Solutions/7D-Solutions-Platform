# bd-1p2 Preparation Notes

**Bead:** bd-1p2 (bd-15.2) - Deterministic Idempotency Key Spec (Cross-Module Contract)
**Agent:** EmeraldBear
**Status:** Preparation (blocked by bd-7gl)
**Date:** 2026-02-15

---

## Objective

Define canonical deterministic idempotency key formats for lifecycle-critical operations:
1. Invoice generation
2. Invoice attempts
3. Payment attempts
4. Ledger postings
5. Notification emission

**Requirements:**
- Deterministic (same inputs → same key)
- Replay-safe (can be recomputed from event data)
- Unique per operation
- Support exactly-once semantics

---

## Current Idempotency Patterns in Codebase

### 1. AR HTTP Idempotency (modules/ar/src/idempotency.rs)
**Pattern:** Client-provided header
```
Idempotency-Key: <client-generated-uuid>
```
**Storage:** `ar_idempotency_keys` table
**Scope:** Per app_id (multi-tenant)
**TTL:** 24 hours
**Mechanism:** SHA-256 hash of key + request body

**Pros:**
- Client controls retry safety
- Works for any HTTP endpoint

**Cons:**
- Not deterministic (client must store key)
- Requires HTTP layer coordination

### 2. GL Event-Based Idempotency (modules/gl/src/repos/journal_repo.rs)
**Pattern:** source_event_id
```sql
CREATE TABLE journal_entries (
    source_event_id UUID NOT NULL UNIQUE,
    ...
);
```
**Mechanism:** UNIQUE constraint → duplicate inserts fail gracefully
**Scope:** Per event (event_id is globally unique)

**Pros:**
- Deterministic (event_id is stable)
- DB-level enforcement
- Replay-safe

**Cons:**
- Only works for event-driven operations

### 3. Event Processing Idempotency (all modules)
**Pattern:** processed_events table
```sql
CREATE TABLE <module>_processed_events (
    event_id UUID NOT NULL UNIQUE,
    ...
);
```
**Mechanism:** INSERT before processing, UNIQUE violation → skip

**Pros:**
- Standard pattern across modules
- Replay-safe

---

## Proposed Idempotency Key Formats

### 1. Invoice Generation
**Context:** Subscription billing cycle → create invoice
**Deterministic Key:**
```
invoice:gen:{tenant_id}:{subscription_id}:{period_start}:{period_end}
```

**Example:**
```
invoice:gen:tenant-123:sub-456:2026-02-01T00:00:00Z:2026-03-01T00:00:00Z
```

**Guarantees:**
- Exactly one invoice per subscription per billing period
- Replay-safe (period boundaries are stable)
- Deterministic (no random components)

**Implementation:**
- Store in `ar_invoice_attempts.idempotency_key` or separate table
- UNIQUE constraint on (tenant_id, subscription_id, period_start, period_end)

---

### 2. Invoice Attempt (AR Retry Windows)
**Context:** Retry invoice payment collection (0d, +3d, +7d)
**Deterministic Key:**
```
invoice:attempt:{tenant_id}:{invoice_id}:{attempt_no}
```

**Example:**
```
invoice:attempt:tenant-123:inv-789:0
invoice:attempt:tenant-123:inv-789:1
invoice:attempt:tenant-123:inv-789:2
```

**Guarantees:**
- Exactly one attempt per retry window
- Deterministic (attempt_no is 0, 1, 2)
- Replay-safe

**Implementation:**
- Store in `ar_invoice_attempts.idempotency_key`
- UNIQUE constraint on (invoice_id, attempt_no) - already enforced by bd-7gl

---

### 3. Payment Attempt (Payments Module)
**Context:** PSP payment execution attempt
**Deterministic Key:**
```
payment:attempt:{tenant_id}:{invoice_id}:{attempt_no}
```

**Example:**
```
payment:attempt:tenant-123:inv-789:0
payment:attempt:tenant-123:inv-789:1
```

**Guarantees:**
- Exactly one PSP call per invoice attempt
- Deterministic
- Replay-safe

**Implementation:**
- Store in `payment_attempts.idempotency_key`
- UNIQUE constraint on (invoice_id, attempt_no) - already enforced by bd-7gl
- Also send to PSP as `Idempotency-Key` header (PSP-level deduplication)

---

### 4. Ledger Posting
**Context:** AR → GL posting for invoice finalization
**Deterministic Key:**
```
ledger:post:{tenant_id}:{source_event_id}
```

**Example:**
```
ledger:post:tenant-123:550e8400-e29b-41d4-a716-446655440000
```

**Guarantees:**
- Exactly one ledger post per AR event
- Deterministic (source_event_id is stable)
- Replay-safe

**Implementation:**
- **Already implemented** via `journal_entries.source_event_id UNIQUE`
- No code changes needed (just document the pattern)

---

### 5. Notification Emission
**Context:** Send email/webhook on lifecycle events
**Deterministic Key:**
```
notification:{tenant_id}:{entity_type}:{entity_id}:{event_type}:{occurrence_index}
```

**Example:**
```
notification:tenant-123:invoice:inv-789:invoice.finalized:0
notification:tenant-123:payment:pay-456:payment.succeeded:0
```

**Guarantees:**
- Exactly one notification per lifecycle event occurrence
- Deterministic
- Replay-safe

**Notes:**
- `occurrence_index` handles rare cases where same event_type fires multiple times
- Usually 0 (first occurrence)

**Implementation:**
- Store in notifications module (TBD - may not exist yet)
- UNIQUE constraint on (tenant_id, entity_type, entity_id, event_type, occurrence_index)

---

## Cross-Module Contract

### Standard Format
```
{operation}:{action}:{tenant_id}:{entity_identifiers}:{attempt_or_index}
```

**Components:**
- `operation`: Domain (invoice, payment, ledger, notification)
- `action`: Verb (gen, attempt, post, emit)
- `tenant_id`: Multi-tenant isolation
- `entity_identifiers`: Stable entity IDs (subscription_id, invoice_id, etc.)
- `attempt_or_index`: Retry number or occurrence index

### Encoding Rules
1. **Use colons** as delimiters (consistent)
2. **Lowercase** operation and action
3. **ISO 8601** for timestamps (if needed)
4. **No random UUIDs** (breaks determinism)
5. **Stable identifiers only** (no ephemeral session IDs)

---

## Implementation Checklist

### For bd-1p2:
- [ ] Create contract specification document (similar to GL contracts)
- [ ] Define key format for each operation
- [ ] Add validation functions (parse and validate key format)
- [ ] Add key generation functions (deterministic builders)
- [ ] Unit tests for key generation (same inputs → same key)
- [ ] Document replay scenarios (prove determinism)
- [ ] Cross-module examples (show usage in AR, Payments, GL, Notifications)

### Acceptance Criteria:
- [ ] Specification document exists in `docs/contracts/idempotency-keys-v1.md`
- [ ] Rust implementation with builders and validators
- [ ] Unit tests prove determinism (10+ test cases)
- [ ] Integration examples for all 5 operation types
- [ ] ChatGPT approval via PearlOwl

---

## Dependencies

**Blocked by:**
- bd-7gl (Attempt Ledgers + DB Uniqueness) - provides attempt_no foundation

**Blocks:**
- bd-138 (Subscriptions Guards)
- bd-1w7 (AR Invoice Guards)
- bd-3lm (Payments Attempt Guards)
- bd-184 (Subscriptions Cycle Gating)

---

## References

- ADR-015: Phase 15 Enforcement Posture (Option B-lite)
- docs/PHASE-15-COORDINATION-RULES.md
- modules/ar/docs/IDEMPOTENCY_AND_EVENTS.md
- modules/ar/src/idempotency.rs
- modules/gl/src/repos/journal_repo.rs (source_event_id pattern)

---

## Notes

**ChatGPT Quote (via ADR-015):**
> "If you encode those as acceptance checks across 15.3x–15.6x, the rest of Phase 15 will stay tight."

**Key Insight:**
Current codebase has 3 different idempotency patterns (HTTP headers, source_event_id, processed_events). bd-1p2 must unify these into a single deterministic specification that works across all lifecycle operations.

**Risk:**
If keys are not truly deterministic, duplicate operations can slip through during replays or concurrent execution. Must prove determinism via tests.

---

**Prepared by:** EmeraldBear
**Next Action:** Claim bd-1p2 when bd-7gl completes
