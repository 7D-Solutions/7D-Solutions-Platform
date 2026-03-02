# ADR-016: Event Contract Conventions

**Date:** 2026-03-02
**Status:** Accepted
**Deciders:** Platform Orchestrator, Engineering Team
**Technical Story:** bd-3f7q7 — Phase 57 EVT0a

## Context and Problem Statement

The platform is expanding from its core financial modules (AR, GL, Payments) into Notifications, Numbering, Document Management, Workflow, and Identity extensions.  Each new service needs to emit and consume events.  Without a single authoritative set of conventions, each scaffold would invent its own envelope shape, naming scheme, idempotency rules, and consumer expectations — leading to brittle cross-service integration.

Existing conventions are scattered across:
- `platform/event-bus/src/envelope/mod.rs` (Rust struct)
- `docs/architecture/EVENT-TAXONOMY.md` (naming)
- `contracts/events/README.md` (envelope table, idempotency)
- `docs/consumer-guide/CG-EVENTS.md` (consumer guide)

This ADR consolidates them into one canonical reference.

## Decision Outcome

All conventions are codified in the **`platform_contracts`** crate (`platform/platform-contracts`) and this document.  Every service scaffold must depend on `platform_contracts` and follow these rules.

---

## 1. EventEnvelope — Required Fields

The canonical envelope is `event_bus::EventEnvelope<T>` (re-exported by `platform_contracts`).

### Always Required

| Field | Type | Description |
|-------|------|-------------|
| `event_id` | UUID | Unique per event.  Idempotency / dedupe key. |
| `event_type` | String | `{entity}.{action}` — see §2. |
| `occurred_at` | DateTime\<Utc\> | When the event was generated. |
| `tenant_id` | String | Multi-tenant isolation.  Never empty. |
| `source_module` | String | Producing module name (e.g. `"ar"`). |
| `source_version` | String | SemVer of the producing module. |
| `schema_version` | String | Schema version of the payload. |
| `replay_safe` | bool | Whether the event can be safely replayed. |
| `mutation_class` | String | One of the 7 canonical classes — see §3. |
| `payload` | T (Serialize) | Event-specific data. |

### Strongly Recommended

| Field | When to set |
|-------|-------------|
| `correlation_id` | Always, for any user-initiated or business-transaction-scoped event. |
| `causation_id` | When the event was caused by another event or command. |
| `actor_id` + `actor_type` | When the initiator is known (`"User"`, `"Service"`, `"System"`). |

### Conditional

| Field | Condition |
|-------|-----------|
| `merchant_context` | **Required** for financial modules + financial mutation classes. |
| `reverses_event_id` | Set on compensating transactions. |
| `supersedes_event_id` | Set on corrections. |
| `trace_id` | Set when distributed tracing is active. |
| `side_effect_id` | Set for non-idempotent external side effects. |

---

## 2. Event Naming & Versioning

### NATS Subject

```
{module}.events.{event_type}
```

Example: `ar.events.invoice.created`, `workflow.events.task.completed`

### Event Type Format

```
{entity}.{action}[.{qualifier}]
```

- Lowercase, dot-delimited, singular entity names.
- **Facts** use past tense: `invoice.created`, `payment.succeeded`.
- **Commands** use `.requested` suffix: `payment.collection.requested`.

See `platform_contracts::event_naming` for helpers and validation.

### Schema Versioning

- Additive changes (new optional field) → keep same major version.
- Breaking changes (remove/rename/retype field) → bump major version, create new `{event}-v{N}.json` schema file, set `schema_version` to new value.
- Producers must emit the old AND new event types during a migration window.
- Consumers must handle older schema versions until explicit cutover.

### Compatibility Rules

1. Never remove fields from event payloads.
2. Only add fields with safe defaults.
3. Breaking change → new event type OR bump `schema_version`.

---

## 3. Mutation Classes

See `platform_contracts::mutation_classes` for constants.

| Class | Meaning | Idempotent? |
|-------|---------|-------------|
| `DATA_MUTATION` | Financial / audit mutation | Yes |
| `REVERSAL` | Compensating transaction | Yes |
| `CORRECTION` | Superseding correction | Yes |
| `SIDE_EFFECT` | Non-idempotent external action | No |
| `QUERY` | Read-only operation | Yes |
| `LIFECYCLE` | Entity lifecycle transition | Yes |
| `ADMINISTRATIVE` | Configuration / setup change | Yes |

---

## 4. Idempotency Key Rules (Command APIs)

See `platform_contracts::idempotency` for helpers.

### Key Format

```
{domain}:{operation}:{tenant_id}:{entity_id}[:{qualifier}]
```

- **Deterministic**: same inputs → same key.
- **Tenant-scoped**: never share keys across tenants.
- **Grain-appropriate**: key boundary matches operation boundary.

### Storage

Each module maintains a `{module}_idempotency_keys` table with `UNIQUE (app_id, idempotency_key)`.

### TTL

Default: **24 hours**.  Modules may extend but never shorten below 24h.

### Replay Behavior

1. Duplicate key within TTL → return stored response verbatim.
2. Do not re-execute the operation.
3. Log `idempotency.replay` for observability.

### HTTP Header

`Idempotency-Key: <key>` — optional.  If absent, no replay protection.

---

## 5. Consumer Dedupe & Replay Invariants

See `platform_contracts::consumer` for constants.

### Dedupe Key

**`event_id`** (UUID).  Consumers persist processed IDs in `processed_events`:

```sql
CREATE TABLE processed_events (
    id           SERIAL PRIMARY KEY,
    event_id     UUID NOT NULL UNIQUE,
    event_type   VARCHAR(255) NOT NULL,
    processed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    processor    VARCHAR(100) NOT NULL
);
```

### Processing Algorithm

1. Receive event.
2. Check `processed_events` for `event_id` → if found, skip.
3. Apply business logic.
4. INSERT into `processed_events` — **same transaction** as step 3.
5. Commit.

Steps 3-5 must be atomic to prevent double-processing.

### Replay Safety

- `replay_safe: true` → consumer may safely re-process (but dedupe catches it).
- `replay_safe: false` → consumer must NOT trigger side effects (email, SMS, webhook) on replay.

### Ordering

Consumers must not assume ordered delivery.  Use `causation_id` and/or `occurred_at` to reconstruct order when necessary.

---

## References

- `platform/event-bus/src/envelope/mod.rs` — canonical Rust struct
- `platform/platform-contracts/` — convention types and constants
- `docs/architecture/EVENT-TAXONOMY.md` — naming taxonomy
- `contracts/events/README.md` — event schema catalog
- `docs/consumer-guide/CG-EVENTS.md` — consumer guide
