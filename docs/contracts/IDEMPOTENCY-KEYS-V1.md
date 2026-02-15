# Idempotency Keys Specification v1.0

**Version:** 1.0
**Status:** Canonical
**Phase:** 15 - Billing Lifecycle Hardening
**Created:** 2026-02-15
**Author:** EmeraldBear
**Bead:** bd-1p2 (bd-15.2)

---

## Purpose

This specification defines **deterministic idempotency key formats** for all lifecycle-critical operations across the billing platform. These keys ensure exactly-once semantics for side effects (PSP calls, event emissions, ledger postings, notifications) under replay, retry, and concurrent execution scenarios.

---

## Core Principles

### 1. Deterministic
**Same inputs MUST produce the same key.**
- No random UUIDs
- No timestamps (unless part of stable entity identity)
- No ephemeral session IDs
- Keys must be recomputable from event/entity data

### 2. Grain-Appropriate
**Keys MUST match the correct operation boundary.**
- Invoice generation: per subscription per billing cycle
- Invoice attempt: per invoice per retry window
- Payment attempt: per payment per retry window
- Ledger posting: per source event
- Notification: per event occurrence

### 3. Module-Local
**Key generation MUST NOT cross module boundaries.**
- Each module implements its own key builders
- No shared idempotency library
- Contracts define formats, modules implement them

### 4. DB-Enforced
**Keys MUST be enforced via UNIQUE constraints.**
- Duplicate key insertion → DB-level rejection
- Application code trusts DB enforcement
- No application-level locking for idempotency

---

## Standard Format

All idempotency keys follow this pattern:

```
{operation}:{tenant_id}:{entity_identifiers}:{attempt_or_occurrence}
```

**Components:**
- `operation`: Domain-specific operation type (lowercase, no spaces)
- `tenant_id`: Multi-tenant isolation (app_id or tenant_id depending on module)
- `entity_identifiers`: Colon-separated stable entity IDs
- `attempt_or_occurrence`: Retry number (0-based) or occurrence index

**Encoding Rules:**
- Use colons (`:`) as delimiters
- Lowercase operation names
- ISO 8601 format for dates (if included): `YYYY-MM-DD`
- No spaces, no special characters (alphanumeric + `-` + `_` + `:` only)

---

## Idempotency Key Formats

### 1. Invoice Generation

**Operation:** Subscription → Invoice creation
**Module:** AR (Accounts Receivable)
**Format:**
```
invoice:gen:{app_id}:{subscription_id}:{cycle_start}:{cycle_end}
```

**Example:**
```
invoice:gen:app-demo:sub-123:2026-02-01:2026-03-01
```

**Guarantees:**
- Exactly one invoice per subscription per billing cycle
- Deterministic (cycle boundaries are stable)
- Replay-safe

**Implementation Location:**
- Module: `modules/ar`
- Function: `generate_invoice_generation_key(app_id, subscription_id, cycle_start, cycle_end)`
- Storage: TBD (may be in subscriptions or invoices table)

**Notes:**
- Cycle dates MUST be normalized to date-only (no time component)
- Use ISO 8601 date format: `YYYY-MM-DD`

---

### 2. Invoice Attempt (AR Collection Retry)

**Operation:** Invoice payment collection attempt
**Module:** AR (Accounts Receivable)
**Format:**
```
invoice:attempt:{app_id}:{invoice_id}:{attempt_no}
```

**Example:**
```
invoice:attempt:app-demo:inv-456:0
invoice:attempt:app-demo:inv-456:1
invoice:attempt:app-demo:inv-456:2
```

**Guarantees:**
- Exactly one attempt per invoice per retry window
- Deterministic (attempt_no is 0, 1, 2 for windows 0d, +3d, +7d)
- Replay-safe

**Implementation Location:**
- Module: `modules/ar`
- Function: `generate_invoice_attempt_key(app_id, invoice_id, attempt_no)`
- Storage: `ar_invoice_attempts.idempotency_key` (UNIQUE constraint via (app_id, invoice_id, attempt_no))

**Retry Windows:**
- Attempt 0: Day 0 (invoice due date)
- Attempt 1: Day 3 (due date + 3 days)
- Attempt 2: Day 7 (due date + 7 days)

**Notes:**
- attempt_no is 0-based (starts at 0)
- UNIQUE constraint on (app_id, invoice_id, attempt_no) prevents duplicates at DB level
- Side effects (PSP call, event emission) only occur on successful INSERT

---

### 3. Payment Attempt (PSP Execution)

**Operation:** Payment Service Provider (PSP) payment execution
**Module:** Payments
**Format:**
```
payment:attempt:{app_id}:{payment_id}:{attempt_no}
```

**Example:**
```
payment:attempt:app-demo:pay-789:0
payment:attempt:app-demo:pay-789:1
```

**Guarantees:**
- Exactly one PSP call per payment per attempt window
- Deterministic (attempt_no matches AR invoice attempt)
- Replay-safe
- PSP-level deduplication (key sent to PSP as Idempotency-Key header)

**Implementation Location:**
- Module: `modules/payments`
- Function: `generate_payment_attempt_key(app_id, payment_id, attempt_no)`
- Storage: `payment_attempts.idempotency_key` (UNIQUE constraint via (app_id, payment_id, attempt_no))

**PSP Integration:**
- Send key to PSP in `Idempotency-Key` HTTP header
- PSP performs its own deduplication using this key
- If PSP rejects duplicate, treat as successful no-op

**Notes:**
- payment_id may be a UUID (distinct from invoice_id)
- attempt_no aligns with AR invoice attempt_no (0, 1, 2)
- UNKNOWN state blocks retry attempts (reconciliation required first)

---

### 4. Ledger Posting (AR → GL)

**Operation:** Journal entry creation from AR event
**Module:** GL (General Ledger)
**Format:**
```
{source_event_id}
```

**Example:**
```
550e8400-e29b-41d4-a716-446655440000
```

**Guarantees:**
- Exactly one journal entry per AR event
- Deterministic (event_id is globally unique and stable)
- Replay-safe

**Implementation Location:**
- Module: `modules/gl`
- Function: Already implemented via `journal_entries.source_event_id`
- Storage: `journal_entries.source_event_id` (UNIQUE constraint)

**Notes:**
- **This pattern is ALREADY IMPLEMENTED** (Phase 9)
- No code changes needed for bd-1p2
- Format is simply the UUID of the source event
- UNIQUE constraint prevents duplicate postings

**Rationale:**
- Event IDs are already globally unique
- No need for composite key
- Simple, proven pattern

---

### 5. Notification Emission

**Operation:** Send notification (email, webhook, push) on lifecycle event
**Module:** Notifications
**Format:**
```
notification:{tenant_id}:{entity_type}:{entity_id}:{event_type}:{occurrence}
```

**Example:**
```
notification:tenant-123:invoice:inv-456:invoice.finalized:0
notification:tenant-123:payment:pay-789:payment.succeeded:0
notification:tenant-123:subscription:sub-123:subscription.suspended:0
```

**Guarantees:**
- Exactly one notification per lifecycle event occurrence
- Deterministic (stable entity and event type)
- Replay-safe

**Implementation Location:**
- Module: `modules/notifications`
- Function: `generate_notification_key(tenant_id, entity_type, entity_id, event_type, occurrence)`
- Storage: TBD (notifications module may not have persistence yet)

**Notes:**
- `occurrence` is usually 0 (first occurrence of this event)
- Handles rare cases where same event_type fires multiple times
- If notifications module doesn't have persistence, store in event outbox

---

## Implementation Guidelines

### Key Builder Functions

Each module MUST implement deterministic key builders:

```rust
// AR Module: modules/ar/src/idempotency.rs
pub fn generate_invoice_generation_key(
    app_id: &str,
    subscription_id: i32,
    cycle_start: NaiveDate,
    cycle_end: NaiveDate,
) -> String {
    format!(
        "invoice:gen:{}:{}:{}:{}",
        app_id,
        subscription_id,
        cycle_start.format("%Y-%m-%d"),
        cycle_end.format("%Y-%m-%d")
    )
}

pub fn generate_invoice_attempt_key(
    app_id: &str,
    invoice_id: i32,
    attempt_no: i32,
) -> String {
    format!(
        "invoice:attempt:{}:{}:{}",
        app_id,
        invoice_id,
        attempt_no
    )
}

// Payments Module: modules/payments/src/idempotency.rs
pub fn generate_payment_attempt_key(
    app_id: &str,
    payment_id: Uuid,
    attempt_no: i32,
) -> String {
    format!(
        "payment:attempt:{}:{}:{}",
        app_id,
        payment_id,
        attempt_no
    )
}

// Notifications Module: modules/notifications/src/idempotency.rs
pub fn generate_notification_key(
    tenant_id: &str,
    entity_type: &str,
    entity_id: &str,
    event_type: &str,
    occurrence: i32,
) -> String {
    format!(
        "notification:{}:{}:{}:{}:{}",
        tenant_id,
        entity_type,
        entity_id,
        event_type,
        occurrence
    )
}
```

### Usage Pattern

**Step 1: Generate key**
```rust
let key = generate_invoice_attempt_key(&app_id, invoice_id, attempt_no);
```

**Step 2: Insert attempt with key**
```rust
let result = sqlx::query(
    "INSERT INTO ar_invoice_attempts
     (app_id, invoice_id, attempt_no, status, idempotency_key)
     VALUES ($1, $2, $3, $4, $5)"
)
.bind(&app_id)
.bind(invoice_id)
.bind(attempt_no)
.bind("attempting")
.bind(&key)
.execute(&pool)
.await;
```

**Step 3: Handle UNIQUE violation**
```rust
match result {
    Ok(_) => {
        // New attempt created → proceed with side effects
        call_psp(&invoice).await?;
        emit_event(&outbox, &invoice).await?;
    }
    Err(sqlx::Error::Database(e)) if e.constraint() == Some("unique_invoice_attempt") => {
        // Duplicate attempt → deterministic no-op
        return Ok(AlreadyProcessed);
    }
    Err(e) => return Err(e.into()),
}
```

---

## Validation and Testing

### Determinism Tests

Each key builder MUST have tests proving determinism:

```rust
#[test]
fn test_invoice_attempt_key_determinism() {
    let app_id = "app-demo";
    let invoice_id = 123;
    let attempt_no = 1;

    let key1 = generate_invoice_attempt_key(app_id, invoice_id, attempt_no);
    let key2 = generate_invoice_attempt_key(app_id, invoice_id, attempt_no);

    assert_eq!(key1, key2);
    assert_eq!(key1, "invoice:attempt:app-demo:123:1");
}
```

### Replay Safety Tests

Tests MUST prove keys are stable under replay:

```rust
#[tokio::test]
async fn test_duplicate_attempt_rejected() {
    let pool = get_test_pool().await;

    // First attempt: succeeds
    let result1 = insert_attempt(&pool, "app-demo", 123, 0).await;
    assert!(result1.is_ok());

    // Second attempt (replay): rejected via UNIQUE constraint
    let result2 = insert_attempt(&pool, "app-demo", 123, 0).await;
    assert!(matches!(result2, Err(DuplicateAttempt)));
}
```

### Cross-Module Consistency Tests

Tests MUST verify keys align across module boundaries:

```rust
#[tokio::test]
async fn test_ar_payment_key_consistency() {
    let app_id = "app-demo";
    let invoice_id = 123;
    let payment_id = Uuid::new_v4();
    let attempt_no = 1;

    // AR creates invoice attempt with key
    let ar_key = ar::generate_invoice_attempt_key(app_id, invoice_id, attempt_no);

    // Payments creates payment attempt with aligned attempt_no
    let payment_key = payments::generate_payment_attempt_key(app_id, payment_id, attempt_no);

    // Keys are different (different entities) but attempt_no aligns
    assert_ne!(ar_key, payment_key);
    assert!(ar_key.ends_with(":1"));
    assert!(payment_key.ends_with(":1"));
}
```

---

## Migration Path

### Existing Systems

For modules with existing idempotency patterns:

**AR HTTP Idempotency (ar_idempotency_keys table):**
- **Keep existing** for HTTP-level deduplication
- **Add new** attempt-level keys (this spec)
- Two layers: HTTP (24h cache) + Attempt (permanent ledger)

**GL Event Deduplication (source_event_id):**
- **Already compliant** with this spec
- No changes needed

**Event Processing (processed_events tables):**
- **Keep existing** for consumer-level deduplication
- **Add new** for operation-level idempotency
- Two layers: Consumer (event replay) + Operation (side effect)

### New Systems

For new lifecycle operations:
1. Generate key using specified format
2. Store in attempt ledger with UNIQUE constraint
3. Trust DB enforcement (no app-level locking)
4. Handle UNIQUE violations as deterministic no-ops

---

## Error Handling

### UNIQUE Constraint Violations

**Behavior:** Treat as successful no-op (deterministic)

```rust
match insert_attempt_with_key(&pool, key).await {
    Ok(_) => {
        // New attempt → proceed with side effects
        Ok(AttemptCreated)
    }
    Err(e) if is_unique_violation(&e) => {
        // Duplicate → deterministic no-op
        Ok(AlreadyProcessed)
    }
    Err(e) => Err(e),
}
```

**Rationale:**
- Duplicate key = duplicate operation trigger
- DB has already rejected the duplicate
- Return success (operation is idempotent)

### Invalid Key Formats

**Behavior:** Fail fast at key generation time

```rust
pub fn generate_invoice_attempt_key(
    app_id: &str,
    invoice_id: i32,
    attempt_no: i32,
) -> Result<String, KeyGenerationError> {
    if app_id.is_empty() {
        return Err(KeyGenerationError::EmptyAppId);
    }
    if attempt_no < 0 || attempt_no > 2 {
        return Err(KeyGenerationError::InvalidAttemptNo(attempt_no));
    }

    Ok(format!("invoice:attempt:{}:{}:{}", app_id, invoice_id, attempt_no))
}
```

---

## Observability

### Metrics

Track idempotency key behavior:

**Key Collision Rate:**
- Metric: `idempotency_key_collisions_total{operation, module}`
- Alert: Rate > 1% (indicates non-deterministic keys or replay storm)

**Duplicate Operation Rate:**
- Metric: `duplicate_operations_total{operation, module}`
- Info: Normal rate is 0.1-1% (retries, replays)

**Key Generation Errors:**
- Metric: `key_generation_errors_total{operation, module, error_type}`
- Alert: Any errors (should be zero)

### Logs

Log key generation and duplicate detection:

```rust
tracing::info!(
    key = %key,
    app_id = %app_id,
    invoice_id = %invoice_id,
    attempt_no = %attempt_no,
    "Generated invoice attempt key"
);

tracing::warn!(
    key = %key,
    "Duplicate attempt detected (UNIQUE violation), treating as no-op"
);
```

---

## Security Considerations

### Key Predictability

**Risk:** Deterministic keys are predictable by definition.

**Mitigation:**
- Keys are internal (not exposed to users)
- Access to attempt ledgers requires authentication
- Predictability is required for determinism (not a bug)

### Key Tampering

**Risk:** Attacker modifies key to bypass idempotency.

**Mitigation:**
- Keys are server-generated (clients never provide them)
- UNIQUE constraints enforce DB-level integrity
- Application code regenerates keys from stable inputs

---

## Version History

### v1.0 (2026-02-15)
- Initial specification
- 5 key formats defined (invoice gen, invoice attempt, payment attempt, ledger post, notification)
- Determinism, grain-appropriateness, module-locality, DB-enforcement principles
- Implementation guidelines and testing requirements
- Phase 15 (bd-1p2) baseline

---

## References

- **ADR-015:** Phase 15 Enforcement Posture (Option B-lite)
- **PHASE-15-COORDINATION-RULES.md:** Mutation Ownership + Exactly-Once Rules
- **bd-7gl:** Attempt Ledgers + DB Uniqueness (foundation)
- **modules/ar/docs/IDEMPOTENCY_AND_EVENTS.md:** Existing AR HTTP idempotency
- **modules/gl/src/repos/journal_repo.rs:** GL source_event_id pattern

---

## Approval

**Status:** ✅ Ready for PearlOwl review → ChatGPT approval

**Next Beads Using This Spec:**
- bd-138 (bd-15.3a): Subscriptions Guards (invoice generation keys)
- bd-1w7 (bd-15.3b): AR Invoice Guards (invoice attempt keys)
- bd-3lm (bd-15.3c): Payments Attempt Guards (payment attempt keys)
- bd-184 (bd-15.4a): Subscriptions Cycle Gating (invoice generation keys)

---

**Document Owner:** EmeraldBear
**Bead:** bd-1p2 (bd-15.2)
**Phase:** 15 - Billing Lifecycle Hardening
