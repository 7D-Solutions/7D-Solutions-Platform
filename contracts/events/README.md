# Event Contracts — 7D Solutions Platform

## Transport Model

Events are published over the platform event bus (e.g., NATS or equivalent).

- Events are immutable.
- Producers do not call consumers directly.
- Event bus provides decoupled, asynchronous communication between modules.

## Event Naming Convention

Use dot notation.

**Format:** `<domain>.<entity>.<action>`

**Examples:**
- `gl.posting.requested`
- `gl.posting.accepted`
- `gl.posting.rejected`
- `ar.invoice.created`
- `ar.payment.received`

## Idempotency Rule

`event_id` is globally unique and serves as the idempotency key.

- Consumers MUST reject duplicate `event_id`.
- Re-processing the same event must be safe and produce the same outcome.
- Producers should generate `event_id` using UUIDv4 or equivalent.

## Envelope Standard

All events MUST include these top-level fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `event_id` | string (uuid) | Yes | Unique event identifier (idempotency key) |
| `occurred_at` | string (date-time) | Yes | ISO 8601 timestamp when event was generated |
| `tenant_id` | string | Yes | Tenant identifier for multi-tenant isolation |
| `source_module` | string | Yes | Module that generated the event (e.g., "ar") |
| `source_version` | string | Yes | Semantic version of the source module |
| `correlation_id` | string | No | Links related events in a business transaction |
| `causation_id` | string | No | Links this event to the command/event that caused it |
| `payload` | object | Yes | Event-specific data (schema varies by event type) |

## GL Posting Rule

**Critical constraint for accounting integrity:**

- AR (and all future modules) MUST emit `gl.posting.requested` events.
- Direct database writes to GL tables are **forbidden**.
- Only the GL module is authorized to create journal entries.
- All accounting postings flow through the event bus.

This ensures:
- Auditability: All postings are traced through events
- Validation: GL enforces balanced entries and account validity
- Decoupling: Modules don't depend on GL database schema

---

## Available Event Schemas

### GL Events

- **gl-posting-request.v1.json** - Request a GL posting
- **gl-posting-accepted.v1.json** - GL posting was accepted and processed
- **gl-posting-rejected.v1.json** - GL posting was rejected (validation failed)

### Subscriptions Events

- **subscriptions-created.v1.json** - New subscription created
- **subscriptions-paused.v1.json** - Subscription paused
- **subscriptions-resumed.v1.json** - Paused subscription resumed
- **subscriptions-billrun-executed.v1.json** - Billing cycle executed

**Subscriptions State Machine:**
```
active → paused → resumed → cancelled
```

Note: Cancelled subscriptions cannot be resumed.
