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
