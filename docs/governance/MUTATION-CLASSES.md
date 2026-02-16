# Mutation Class Registry

**Phase 16: Financial Controls & Replay Safety**

## Purpose

Every event in the system MUST declare a `mutation_class` to indicate its impact on system state and replayability characteristics. This classification enables:

1. **Audit Controls**: Prevent illegal updates to financial/audit records
2. **Replay Safety**: Determine which events can be safely replayed
3. **Concurrency Control**: Identify events requiring serialization
4. **Regulatory Compliance**: Track mutation patterns for SOX/SOC2 compliance

## Classification Rules

### DATA_MUTATION

**Definition:** Events that create or modify financial/audit records in an idempotent manner.

**Characteristics:**
- Deterministic: Same input produces same output
- Idempotent: Can be replayed without side effects
- Financially significant: Creates/modifies AR, GL, or Payments records
- Replay-safe: Yes

**Examples:**
- `ar.invoice.created` - Creates invoice record
- `gl.entry.posted` - Creates journal entry
- `payments.payment.succeeded` - Records payment success
- `subscriptions.subscription.activated` - Activates subscription

**Enforcement:**
- MUST use Guard → Mutation → Outbox discipline
- MUST check for duplicates via processed_events table
- MUST be atomic within transaction

**Policy:** Once created, these events CANNOT be deleted or modified in event log. Corrections must use reversal events.

---

### REVERSAL

**Definition:** Events that compensate for previous DATA_MUTATION events.

**Characteristics:**
- References original event via `reverses_event_id`
- Creates inverse/compensating transaction
- Deterministic and idempotent
- Replay-safe: Yes

**Examples:**
- `gl.entry.reversed` - Reverses journal entry
- `ar.invoice.voided` - Voids invoice
- `payments.refund.issued` - Refunds payment

**Enforcement:**
- MUST populate `reverses_event_id` field
- MUST reference a valid original event
- CANNOT reverse a reversal (no chains)
- Original event's period MUST NOT be closed (Phase 13 governance)

**Policy:** Reversals are append-only. Never modify the original event.

---

### CORRECTION

**Definition:** Events that supersede incorrect/obsolete DATA_MUTATION events.

**Characteristics:**
- References original event via `supersedes_event_id`
- Replaces prior event with corrected version
- Deterministic and idempotent
- Replay-safe: Yes

**Examples:**
- `gl.entry.corrected` - Corrects GL posting error
- `ar.invoice.amended` - Amends invoice details

**Enforcement:**
- MUST populate `supersedes_event_id` field
- MUST reference a valid original event
- Original event MUST be within current/open period
- Correction MUST preserve financial balance

**Policy:** Corrections are append-only. Original event remains in log with `superseded_by` marker.

---

### SIDE_EFFECT

**Definition:** Events that trigger external actions or notifications (non-idempotent side effects).

**Characteristics:**
- Not idempotent: Replay causes duplicate side effects
- External impact: Sends email, SMS, webhook, API call
- Not financially significant
- Replay-safe: No

**Examples:**
- `notifications.email.sent` - Sends email notification
- `notifications.sms.sent` - Sends SMS
- `webhooks.delivered` - Fires webhook to external system
- `integrations.api.called` - Calls third-party API

**Enforcement:**
- MUST use deduplication token (`side_effect_id`)
- MUST check processed_events table before executing
- SHOULD implement idempotency key at receiver (if possible)
- MAY retry failed side effects with same side_effect_id

**Policy:** Side effects MUST be protected by deduplication. Replay protection is CRITICAL.

---

### QUERY

**Definition:** Events representing read operations or queries (observational, no mutations).

**Characteristics:**
- Read-only: No state changes
- Idempotent: Can be replayed freely
- Not financially significant
- Replay-safe: Yes

**Examples:**
- `analytics.report.generated` - Generates report
- `audit.access.logged` - Logs access event
- `cache.refreshed` - Refreshes cache

**Enforcement:**
- SHOULD NOT write to operational tables
- MAY write to audit/analytics tables
- MUST be idempotent

**Policy:** Query events are low-risk and can be replayed without concern.

---

### LIFECYCLE

**Definition:** Events managing entity lifecycle state transitions.

**Characteristics:**
- State machine transitions (e.g., draft → active → closed)
- Idempotent within constraints
- May have financial implications
- Replay-safe: Conditional

**Examples:**
- `subscriptions.cycle.started` - Billing cycle start
- `gl.period.closed` - Period close event
- `ar.invoice.finalized` - Invoice finalization

**Enforcement:**
- MUST enforce state machine constraints
- MUST be idempotent (replaying when already in target state is no-op)
- SHOULD use optimistic locking or version checks

**Policy:** Lifecycle events control workflow gates. Replay safety depends on proper state checks.

---

### ADMINISTRATIVE

**Definition:** Events for configuration, setup, or administrative actions.

**Characteristics:**
- Infrequent: System configuration changes
- Idempotent: Can be replayed safely
- Not financially significant
- Replay-safe: Yes

**Examples:**
- `auth.user.created` - Creates user account
- `config.setting.updated` - Updates configuration
- `accounts.account.created` - Creates chart of accounts entry

**Enforcement:**
- SHOULD use UPSERT semantics (insert or update)
- MUST be idempotent

**Policy:** Administrative events are safe to replay.

---

## Mutation Class Matrix

| Mutation Class   | Idempotent | Replay-Safe | Financial | Dedup Required | Linkage Field         |
|------------------|------------|-------------|-----------|----------------|-----------------------|
| DATA_MUTATION    | Yes        | Yes         | Yes       | Yes            | -                     |
| REVERSAL         | Yes        | Yes         | Yes       | Yes            | reverses_event_id     |
| CORRECTION       | Yes        | Yes         | Yes       | Yes            | supersedes_event_id   |
| SIDE_EFFECT      | No         | No          | No        | Yes            | side_effect_id        |
| QUERY            | Yes        | Yes         | No        | Optional       | -                     |
| LIFECYCLE        | Yes        | Conditional | Maybe     | Yes            | -                     |
| ADMINISTRATIVE   | Yes        | Yes         | No        | Yes            | -                     |

---

## Domain Concept Registry

**Purpose**: Identify core financial domain concepts and their immutability characteristics BEFORE enforcement logic is built. This registry prevents retroactive policy edits after data exists.

### Classification Categories

1. **Strict Immutable**: Cannot be modified after creation; corrections require compensating transactions
2. **Lifecycle Managed**: Can be amended during open/draft state; becomes immutable after finalization
3. **Audit Trail Only**: Modifications tracked but not restricted

---

### Core Domain Concepts

#### 1. Invoices
- **Owner Module**: AR (Accounts Receivable)
- **Policy**: Strict Immutable (once finalized)
- **Compensating Strategy**: REVERSAL required
- **Event Examples**:
  - Creation: `ar.invoice.created` (mutation_class: DATA_MUTATION)
  - Void: `ar.invoice.voided` (mutation_class: REVERSAL)
- **Enforcement**:
  - Draft invoices MAY be modified
  - Finalized invoices MUST NOT be modified
  - Corrections MUST use `ar.invoice.voided` with `reverses_event_id`
  - Period close enforcement (Phase 13): cannot void closed-period invoices

#### 2. Payment Attempts
- **Owner Module**: Payments
- **Policy**: Strict Immutable (once succeeded/failed)
- **Compensating Strategy**: REVERSAL required (refunds)
- **Event Examples**:
  - Success: `payments.payment.succeeded` (mutation_class: DATA_MUTATION)
  - Refund: `payments.refund.issued` (mutation_class: REVERSAL)
- **Enforcement**:
  - Payment attempts are write-once records
  - Refunds MUST reference original payment via `reverses_event_id`
  - Partial refunds create new refund records (not modifications)
  - Full audit trail required for regulatory compliance

#### 3. Journal Entries
- **Owner Module**: GL (General Ledger)
- **Policy**: Strict Immutable (once posted)
- **Compensating Strategy**: REVERSAL required
- **Event Examples**:
  - Posting: `gl.entry.posted` (mutation_class: DATA_MUTATION)
  - Reversal: `gl.entry.reversed` (mutation_class: REVERSAL)
- **Enforcement**:
  - Posted journal entries MUST NOT be modified
  - Corrections MUST use `gl.entry.reversed` with `reverses_event_id`
  - Reversal creates offsetting entry with opposite signs
  - Period close enforcement (Phase 13): cannot reverse closed-period entries
  - Double-entry bookkeeping integrity maintained

#### 4. Subscription Cycles
- **Owner Module**: Subscriptions
- **Policy**: Lifecycle Managed
- **Compensating Strategy**: CORRECTION during open state, REVERSAL after close
- **Event Examples**:
  - Start: `subscriptions.cycle.started` (mutation_class: LIFECYCLE)
  - Amendment: `subscriptions.cycle.amended` (mutation_class: CORRECTION)
  - Close: `subscriptions.cycle.closed` (mutation_class: LIFECYCLE)
- **Enforcement**:
  - Open cycles MAY be amended via `supersedes_event_id`
  - Closed cycles are immutable
  - Billing runs reference cycle state at execution time
  - State transitions enforce: draft → active → closed (no backwards transitions)

#### 5. Reconciliation Artifacts
- **Owner Module**: GL (General Ledger)
- **Policy**: Strict Immutable (once completed)
- **Compensating Strategy**: REVERSAL required (rare; typically indicates accounting error)
- **Event Examples**:
  - Match: `gl.reconciliation.matched` (mutation_class: DATA_MUTATION)
  - Unmatch: `gl.reconciliation.unmatched` (mutation_class: REVERSAL)
- **Enforcement**:
  - Reconciliation matches are write-once
  - Unmatching requires reversal event
  - Bank statement reconciliation must maintain complete audit trail
  - Regulatory requirement: SOX compliance for financial reconciliations

---

### Immutability Policy Matrix

| Domain Concept          | Owner Module  | Immutability Policy    | Compensating Strategy       | Period Close Impact |
|-------------------------|---------------|------------------------|-----------------------------|---------------------|
| Invoices                | AR            | Strict Immutable       | REVERSAL (void)             | Yes - blocks voids  |
| Payment Attempts        | Payments      | Strict Immutable       | REVERSAL (refund)           | No                  |
| Journal Entries         | GL            | Strict Immutable       | REVERSAL (offsetting entry) | Yes - blocks reversal |
| Subscription Cycles     | Subscriptions | Lifecycle Managed      | CORRECTION (open) / REVERSAL (closed) | No |
| Reconciliation Artifacts| GL            | Strict Immutable       | REVERSAL (unmatch)          | Yes - blocks unmatch |

---

### Design Rationale

**Why Classify Domain Concepts?**

1. **Prevent Retroactive Policy Edits**: Establishing immutability rules BEFORE data exists prevents architectural debt
2. **Regulatory Compliance**: SOX, SOC2, and audit requirements demand clear retention and modification policies
3. **Replay Safety**: Strict immutability guarantees enable safe event replay without risk of data corruption
4. **Developer Guidance**: Clear ownership and policies prevent ad-hoc modification patterns

**Failure Mode to Avoid**: Discovering after production that a "temporary" UPDATE was used instead of a compensating transaction, breaking audit trail and replay integrity.

---

## Enforcement Rules

### 1. **Mutation Class is Required**

Every event MUST declare a mutation_class. The EventEnvelope will reject events with `None` or empty mutation_class.

```rust
// VALID
let envelope = EventEnvelope::new(...)
    .with_mutation_class(Some("DATA_MUTATION".to_string()));

// INVALID - will be rejected at validation boundary
let envelope = EventEnvelope::new(...)
    .with_mutation_class(None); // ❌ REJECTED
```

### 2. **Deduplication Protocol**

All events MUST use the processed_events table for idempotency:

```rust
// Check if already processed
if processed_repo::exists(pool, event_id).await? {
    return Err(Error::DuplicateEvent(event_id));
}

// Guard: Validate
// Mutation: Execute state change
// Outbox: Emit event with mutation_class

// Mark as processed
processed_repo::insert(pool, event_id, event_type, handler).await?;
```

### 3. **Financial Event Constraints**

Events with `mutation_class = DATA_MUTATION`, `REVERSAL`, or `CORRECTION` MUST:

- Use transaction boundaries (BEGIN...COMMIT)
- Write to operational tables AND outbox atomically
- Enforce accounting period rules (Phase 13)
- Maintain audit trail (never delete)

### 4. **Reversal Linkage**

Events with `mutation_class = REVERSAL` MUST:

- Populate `reverses_event_id` with the original event's ID
- NOT reverse events whose period is closed
- NOT reverse other reversals (no chains)

### 5. **Side Effect Protection**

Events with `mutation_class = SIDE_EFFECT` MUST:

- Populate `side_effect_id` with a deduplication token
- Check processed_events BEFORE executing side effect
- Use idempotency keys at receiver (if supported)

---

## Validation Rules

The platform enforces mutation_class validation at the outbox boundary:

```rust
// platform/event-bus/src/outbox.rs
fn validate_envelope_fields(envelope: &serde_json::Value) -> Result<(), String> {
    // Required: mutation_class cannot be None or empty
    if let Some(mutation_class) = envelope.get("mutation_class") {
        if mutation_class.is_null() {
            return Err("mutation_class cannot be null".to_string());
        }
        if let Some(s) = mutation_class.as_str() {
            if s.trim().is_empty() {
                return Err("mutation_class cannot be empty string".to_string());
            }
        }
    } else {
        return Err("mutation_class is required".to_string());
    }

    // Validate mutation_class is a known value
    const VALID_CLASSES: &[&str] = &[
        "DATA_MUTATION",
        "REVERSAL",
        "CORRECTION",
        "SIDE_EFFECT",
        "QUERY",
        "LIFECYCLE",
        "ADMINISTRATIVE",
    ];

    if let Some(class) = envelope.get("mutation_class").and_then(|v| v.as_str()) {
        if !VALID_CLASSES.contains(&class) {
            return Err(format!(
                "Invalid mutation_class: '{}'. Must be one of: {:?}",
                class, VALID_CLASSES
            ));
        }
    }

    Ok(())
}
```

---

## Migration Path

### Existing Events (Pre-Phase 16)

For events created before Phase 16, the mutation_class field may be NULL in the database. During replay:

1. **Phase 16 Migration**: Add `mutation_class` column with default NULL for backward compatibility
2. **Runtime Validation**: New events MUST have non-null mutation_class
3. **Backfill** (Future): Assign mutation_class to historical events based on event_type heuristics

### Recommended Backfill Heuristics

```sql
-- Backfill mutation_class for historical events (Phase 16 - Future Work)
UPDATE events_outbox
SET mutation_class = CASE
    WHEN event_type LIKE '%.created' THEN 'DATA_MUTATION'
    WHEN event_type LIKE '%.reversed' THEN 'REVERSAL'
    WHEN event_type LIKE '%.corrected' THEN 'CORRECTION'
    WHEN event_type LIKE 'notifications.%' THEN 'SIDE_EFFECT'
    WHEN event_type LIKE '%.closed' OR event_type LIKE '%.started' THEN 'LIFECYCLE'
    WHEN event_type LIKE 'auth.%' OR event_type LIKE 'accounts.account.%' THEN 'ADMINISTRATIVE'
    ELSE 'QUERY'
END
WHERE mutation_class IS NULL;
```

---

## References

- Phase 13: Accounting Period Close Lifecycle
- Phase 15: Billing Lifecycle Hardening (Deterministic Execution Layer)
- Phase 16: Event Envelope Hardening & Production Readiness
- DOMAIN-OWNERSHIP-REGISTRY.md: Domain ownership and degradation policies
- RETRY-WINDOWS.md: Event processing retry policies
