# Consumer Guide — Events, Outbox & Integrations

> **Who reads this:** Agents building vertical apps on the 7D Platform.
> **What it covers:** NATS event bus, EventEnvelope, the outbox pattern (copy-paste ready), and the Integrations module.
> **Parent:** [PLATFORM-CONSUMER-GUIDE.md](./PLATFORM-CONSUMER-GUIDE.md)

## Contents

1. [NATS Event Bus](#nats-event-bus) — subject naming, EventEnvelope (17 fields), creating envelopes, MerchantContext, idempotency, known subjects, evolution rules
2. [Outbox Pattern — Copy This](#outbox-pattern--copy-this) — SQL migration, `enqueue_event_tx()`, SDK auto-publisher
3. [Integrations Module](#integrations-module) — inbound webhooks, external ID mapping
4. [module.toml — Complete Template](#moduletoml--complete-template) — all supported sections with examples
5. [Gotchas](#gotchas) — common pitfalls with event_type, publishers, and module.toml

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-CONSUMER-GUIDE.md. NATS subjects, EventEnvelope, MerchantContext, outbox migration + enqueue + background publisher, Integrations module. |
| 2.0 | 2026-03-04 | MaroonHarbor | Added all Phase 57+67 event subjects: party contacts/tags, maintenance (17 subjects), auth SoD, notifications (templates, delivery, inbox, DLQ, broadcast). |
| 3.0 | 2026-04-02 | CopperRiver | Fix event_type format to match SDK publisher (full NATS subject). Add complete module.toml template. Add gotchas section. Reference event catalog. |

---

## NATS Event Bus

Source: `platform/event-bus/src/envelope/mod.rs`, `modules/ar/src/events/publisher.rs`, `platform/identity-auth/src/auth/handlers.rs`

Platform uses **NATS JetStream** for async events.

### Subject Naming Convention

**Pattern:** `{module}.{event_name}` — the `event_type` field stored in the outbox IS the full NATS subject.

```
ar.invoice_opened
ar.invoice_paid
ar.invoice_suspended
shipping_receiving.shipment_created
shipping_receiving.shipment_status_changed
auth.user_registered
auth.user_logged_in
yourapp.order_created              ← your events
yourapp.order_completed            ← your events
```

Source: SDK publisher at `platform/platform-sdk/src/publisher.rs` line 168: `bus.publish(&event_type, bytes)` — uses event_type directly as the NATS subject.
Source: AR event constants at `modules/ar/src/events/contracts/invoice_lifecycle.rs`: `EVENT_TYPE_INVOICE_OPENED = "ar.invoice_opened"`.

**The event_type you store in the outbox IS the NATS subject.** The SDK publisher does not add any prefix. Whatever string you put in the `event_type` column is exactly what gets published as the NATS subject.

> **Canonical reference:** See [event-catalog.md](../event-catalog.md) for the complete list of all event subjects across all modules.

### EventEnvelope — Canonical Structure (17 Fields)

Source: `platform/event-bus/src/envelope/mod.rs` → `EventEnvelope<T>`

This is the platform-wide event envelope. **Use the `event-bus` crate — do not reimplement.**

```rust
pub struct EventEnvelope<T> {
    pub event_id: Uuid,                           // Auto-generated. Idempotency key.
    pub event_type: String,                        // Full NATS subject, e.g. "yourapp.order_created"
    pub occurred_at: DateTime<Utc>,                // Auto-generated.
    pub tenant_id: String,                         // Multi-tenant isolation.
    pub source_module: String,                     // e.g. "trashtech"
    pub source_version: String,                    // Default "1.0.0". Use CARGO_PKG_VERSION.
    pub schema_version: String,                    // Default "1.0.0".
    pub trace_id: Option<String>,                  // Distributed tracing.
    pub correlation_id: Option<String>,            // Links events in a business transaction.
    pub causation_id: Option<String>,              // What caused this event.
    pub reverses_event_id: Option<Uuid>,           // Compensating transactions.
    pub supersedes_event_id: Option<Uuid>,         // Corrections.
    pub side_effect_id: Option<String>,            // Side-effect idempotency.
    pub replay_safe: bool,                         // Default true.
    pub mutation_class: Option<String>,            // e.g. "financial", "user-data"
    pub actor_id: Option<Uuid>,                    // Who caused this event.
    pub actor_type: Option<String>,                // "user", "service", "system"
    pub merchant_context: Option<MerchantContext>, // Money-mixing guard. Required for financial.
    pub payload: T,                                // Your event-specific data.
}
```

### Creating an Envelope

```rust
use event_bus::{EventEnvelope, MerchantContext};

// Basic construction — event_type is the full NATS subject
let envelope = EventEnvelope::new(
    tenant_id.to_string(),              // tenant_id
    "your-app".to_string(),            // source_module
    "yourapp.order_created".to_string(), // event_type = full NATS subject
    payload,                            // your struct implementing Serialize
);

// With builder methods
let envelope = EventEnvelope::new(tenant_id, "your-app".into(), "yourapp.order_created".into(), payload)
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_correlation_id(Some(correlation_id))
    .with_causation_id(Some(causation_id))
    .with_mutation_class(Some("operational".to_string()))
    .with_actor(user_id, "user".to_string())
    .with_merchant_context(Some(MerchantContext::Tenant(tenant_id.to_string())));
```

Builder methods available (all return `Self`):
```
.with_source_version(String)
.with_schema_version(String)
.with_trace_id(Option<String>)
.with_correlation_id(Option<String>)
.with_causation_id(Option<String>)
.with_reverses_event_id(Option<Uuid>)
.with_supersedes_event_id(Option<Uuid>)
.with_side_effect_id(Option<String>)
.with_replay_safe(bool)
.with_mutation_class(Option<String>)
.with_actor(Uuid, String)             // use when actor_id and actor_type are known (user-initiated events)
.with_actor_from(Option<Uuid>, Option<String>)  // use for system-initiated events where actor may be None
.with_merchant_context(Option<MerchantContext>)
.with_tracing_context(&TracingContext)
```

### MerchantContext

Source: `platform/event-bus/src/envelope/mod.rs`

```rust
#[serde(tag = "type", content = "id")]
pub enum MerchantContext {
    Tenant(String),  // Your events. Inner value = tenant_id.
    Platform,        // 7D internal. NEVER use this.
}
```

Serialized JSON:
```json
{ "type": "Tenant", "id": "550e8400-..." }
```

**For TrashTech domain events: always use `MerchantContext::Tenant(tenant_id)`.** The `Platform` variant is reserved for 7D internal billing operations (e.g. when the platform invoices a tenant for its own SaaS fees). TrashTech events are never platform-of-record transactions.

Rule: `merchant_context` must match the merchant of record for the transaction. TrashTech charges customers → `Tenant`. 7D charges TrashTech Pro → `Platform` (but you never emit those events).

Required for financial events (invoicing, payments). Optional for non-financial (GPS pings, route updates).

### Idempotency

All events are deduplicated by `event_id`. Your consumer must check and skip already-processed `event_id` values using a `processed_events` table.

### Known NATS Subjects

#### Auth (identity-auth)

| Subject | Trigger |
|---------|---------|
| `auth.user_registered` | User registered |
| `auth.user_logged_in` | Successful login |
| `auth.token_refreshed` | JWT token refreshed |
| `auth.password_reset_requested` | Forgot-password initiated |
| `auth.password_reset_completed` | Password reset completed |
| `auth.sod.policy.upserted` | SoD policy created or updated |
| `auth.sod.policy.deleted` | SoD policy deleted |
| `auth.sod.decision.recorded` | SoD evaluation decision logged |

#### AR

| Subject | Trigger |
|---------|---------|
| `ar.invoice_opened` | Invoice opened |
| `ar.invoice_paid` | Invoice fully paid |
| `ar.invoice_suspended` | Invoice suspended (dunning) |
| `ar.invoice_written_off` | Invoice written off |
| `ar.invoice_settled_fx` | Invoice FX settlement |
| `ar.credit_note_issued` | Credit note issued |
| `ar.payment_allocated` | Payment allocated to invoice |
| `ar.usage_captured` | Usage data captured |
| `ar.usage_invoiced` | Usage invoiced |

#### Payments

| Subject | Trigger |
|---------|---------|
| `payment.succeeded` | Payment gateway success |
| `payment.failed` | Payment gateway failure |

#### Party Master

| Subject | Trigger |
|---------|---------|
| `party.created` | Party created (company or individual) |
| `party.updated` | Party updated |
| `party.deactivated` | Party deactivated |
| `party.events.contact.created` | Contact created on a party |
| `party.events.contact.updated` | Contact updated |
| `party.events.contact.deactivated` | Contact soft-deleted |
| `party.events.contact.primary_set` | Contact set as primary for a role |
| `party.events.tags.updated` | Party tags updated |
| `party.vendor_qualification.created` | Vendor qualification created |
| `party.vendor_qualification.updated` | Vendor qualification updated |
| `party.credit_terms.created` | Credit terms created |
| `party.credit_terms.updated` | Credit terms updated |
| `party.contact_role.created` | Contact role created |
| `party.contact_role.updated` | Contact role updated |
| `party.scorecard.created` | Vendor scorecard created |
| `party.scorecard.updated` | Vendor scorecard updated |

#### Maintenance (17 subjects)

Source: `modules/maintenance/src/events/subjects.rs`

| Subject | Trigger |
|---------|---------|
| `maintenance.work_order.created` | Work order created |
| `maintenance.work_order.status_changed` | Work order status transitioned |
| `maintenance.work_order.completed` | Work order completed |
| `maintenance.work_order.closed` | Work order closed |
| `maintenance.work_order.cancelled` | Work order cancelled |
| `maintenance.work_order.overdue` | Work order marked overdue |
| `maintenance.meter_reading.recorded` | Meter reading recorded |
| `maintenance.plan.due` | Maintenance plan due |
| `maintenance.plan.assigned` | Plan assigned to asset |
| `maintenance.asset.created` | Asset created |
| `maintenance.asset.updated` | Asset updated |
| `maintenance.downtime.recorded` | Downtime event recorded |
| `maintenance.calibration.created` | Calibration created |
| `maintenance.calibration.completed` | Calibration completed |
| `maintenance.calibration.event_recorded` | Calibration event recorded |
| `maintenance.calibration.status_changed` | Calibration status changed |
| `maintenance.asset.out_of_service_changed` | Asset out-of-service status changed |

#### Notifications

| Subject | Trigger |
|---------|---------|
| `notifications.events.template.published` | New template version published |
| `notifications.events.delivery.attempted` | Delivery attempt made |
| `notifications.events.delivery.succeeded` | Delivery succeeded |
| `notifications.delivery.failed` | Delivery failed (exhausted retries) |
| `notifications.events.inbox.message_created` | Inbox message created |
| `notifications.events.inbox.message_read` | Inbox message marked read |
| `notifications.events.inbox.message_unread` | Inbox message marked unread |
| `notifications.events.inbox.message_dismissed` | Inbox message dismissed |
| `notifications.events.inbox.message_undismissed` | Inbox message undismissed |
| `notifications.events.broadcast.created` | Broadcast notification created |
| `notifications.events.broadcast.delivered` | Broadcast delivered |
| `notifications.events.dlq.replayed` | DLQ item replayed |
| `notifications.events.dlq.abandoned` | DLQ item abandoned |

#### Tenant

| Subject | Trigger |
|---------|---------|
| `tenant.provisioned` | New tenant created — subscribe to trigger DB creation |

### Event Evolution Rules

1. Never remove fields from event payloads
2. Only add fields with safe defaults
3. Breaking change → emit new event type OR bump `schema_version`
4. Consumers must handle older schema versions until cutover

---

## Outbox Pattern — Copy This

Source: `modules/ar/db/migrations/20260211000001_create_events_outbox.sql`, `modules/ar/db/migrations/20260216000001_add_envelope_metadata_to_outbox.sql`

### Migration: Create Outbox Tables

Copy this into your first migration for TrashTech:

> **Note on EventEnvelope fields:** `EventEnvelope` has `actor_id`, `actor_type`, and `merchant_context` fields. These are **not** separate outbox columns — they are serialized into the `payload` JSONB by `validate_and_serialize_envelope()`. The individual columns (tenant_id, trace_id, etc.) exist for database-level indexing and querying. The `payload` column carries the full envelope for NATS publishing. If you compare the struct to the SQL and see missing fields, this is why.

> **Note on TIMESTAMP vs TIMESTAMPTZ:** `created_at` and `published_at` use `TIMESTAMP` (no timezone, stored as UTC by convention). `occurred_at` uses `TIMESTAMPTZ` (explicit timezone). This matches the AR source migration. Copy as-is.

```sql
-- events_outbox: Transactional outbox for reliable event publishing
CREATE TABLE events_outbox (
    id SERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    event_type VARCHAR(255) NOT NULL,
    aggregate_type VARCHAR(100) NOT NULL,
    aggregate_id VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    published_at TIMESTAMP,
    -- Envelope metadata (all from EventEnvelope)
    tenant_id VARCHAR(255),
    source_module VARCHAR(100),
    source_version VARCHAR(50),
    schema_version VARCHAR(50),
    occurred_at TIMESTAMPTZ,
    replay_safe BOOLEAN DEFAULT true,
    trace_id VARCHAR(255),
    correlation_id VARCHAR(255),
    causation_id VARCHAR(255),
    reverses_event_id UUID,
    supersedes_event_id UUID,
    side_effect_id VARCHAR(255),
    mutation_class VARCHAR(100)
);

-- Index for unpublished events (background publisher polls this)
CREATE INDEX idx_events_outbox_unpublished ON events_outbox (created_at)
WHERE published_at IS NULL;

-- Index for cleanup queries
CREATE INDEX idx_events_outbox_published ON events_outbox (published_at)
WHERE published_at IS NOT NULL;

-- Index for tenant-scoped queries
CREATE INDEX idx_events_outbox_tenant_id ON events_outbox(tenant_id)
WHERE tenant_id IS NOT NULL;

-- Index for distributed tracing
CREATE INDEX idx_events_outbox_trace_id ON events_outbox(trace_id)
WHERE trace_id IS NOT NULL;

-- processed_events: Idempotent consumer dedup
CREATE TABLE processed_events (
    id SERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    event_type VARCHAR(255) NOT NULL,
    processed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    processor VARCHAR(100) NOT NULL
);

CREATE INDEX idx_processed_events_event_id ON processed_events (event_id);
```

### Outbox Enqueue (Transactional)

Source: `modules/ar/src/events/outbox.rs` → `enqueue_event_tx()`

> **Import note:** `validate_and_serialize_envelope` is in the **platform** `event-bus` crate (`platform/event-bus/src/outbox.rs`), NOT in AR. The import below is correct. AR's `outbox.rs` is the *wrapper* you copy into your module.

```rust
use event_bus::outbox::validate_and_serialize_envelope;

pub async fn enqueue_event_tx<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_type: &str,          // Full NATS subject, e.g. "yourapp.order_created"
    aggregate_type: &str,      // e.g. "order"
    aggregate_id: &str,        // e.g. order UUID
    envelope: &EventEnvelope<T>,
) -> Result<(), sqlx::Error> {
    let payload = validate_and_serialize_envelope(envelope)
        .map_err(|e| sqlx::Error::Encode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Envelope validation failed: {}", e),
        ))))?;

    sqlx::query(
        r#"INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, source_version, schema_version,
            occurred_at, replay_safe, trace_id, correlation_id, causation_id,
            reverses_event_id, supersedes_event_id, side_effect_id, mutation_class
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
    )
    .bind(envelope.event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(payload)
    .bind(&envelope.tenant_id)
    .bind(&envelope.source_module)
    .bind(&envelope.source_version)
    .bind(&envelope.schema_version)
    .bind(envelope.occurred_at)
    .bind(envelope.replay_safe)
    .bind(&envelope.trace_id)
    .bind(&envelope.correlation_id)
    .bind(&envelope.causation_id)
    .bind(&envelope.reverses_event_id)
    .bind(&envelope.supersedes_event_id)
    .bind(&envelope.side_effect_id)
    .bind(&envelope.mutation_class)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
```

### Background Publisher

**The SDK handles this automatically.** When your `module.toml` declares `[events.publish].outbox_table`, the SDK spawns a background publisher that polls the outbox table every 1 second. You do not need to write publisher code.

Source: `platform/platform-sdk/src/publisher.rs` → `run_outbox_publisher()`

For each unpublished event:
1. Read `event_type` from outbox row — this IS the NATS subject
2. Serialize `payload` column to bytes
3. Publish via `event_bus::EventBus` trait to subject = `event_type`
4. Mark as published (`UPDATE {outbox_table} SET published_at = NOW() WHERE event_id = $1`)

**You do NOT need to write a publisher or spawn a background task.** The SDK does this for you based on the `[events.publish]` section in module.toml. If this section is missing, no publisher runs and outbox rows accumulate forever.

To debug stuck events: `SELECT * FROM events_outbox WHERE published_at IS NULL ORDER BY created_at;`

---

## Integrations Module

Source: `modules/integrations/src/`

**Base URL:** `http://7d-integrations:8099`

### Inbound Webhooks (External → Platform)

Source: `modules/integrations/src/http/webhooks.rs` → `inbound_webhook()`

```
POST /api/webhooks/inbound/{system}
x-app-id: <your-app-id>
x-webhook-id: <source-system-event-id>   ← idempotency key (optional, Stripe event ID etc.)
Content-Type: application/json
```

Body: raw JSON from the external system (verbatim). The body is stored as-is and routed by `{system}`.

```json
{
  "id": "evt_1234",
  "type": "customer.updated",
  "data": { "...": "source-system-specific" }
}
```

Response:
```json
{ "status": "accepted", "ingest_id": 42 }
{ "status": "duplicate", "ingest_id": 41 }   ← if x-webhook-id matches prior ingest
```

Use for routing GPS provider webhooks, payment gateway callbacks, or any external event into the platform.

### External ID Mapping

Source: `modules/integrations/src/http/external_refs.rs`, `modules/integrations/src/domain/external_refs/models.rs`

Map your internal IDs to external system IDs:

```
POST   /api/integrations/external-refs
GET    /api/integrations/external-refs/by-entity?entity_type=order&entity_id=<uuid>
GET    /api/integrations/external-refs/by-system?system=stripe&external_id=cus_12345
GET    /api/integrations/external-refs/{id}
PUT    /api/integrations/external-refs/{id}
DELETE /api/integrations/external-refs/{id}
```

Create body:
```json
{
  "entity_type": "order",
  "entity_id": "<order-uuid>",
  "system": "stripe",
  "external_id": "cus_12345",
  "label": "Acme Corp Stripe customer",
  "metadata": { "plan": "enterprise" }
}
```

Response (`ExternalRef`):
```json
{
  "id": 7,
  "app_id": "<your-app-id>",
  "entity_type": "order",
  "entity_id": "<order-uuid>",
  "system": "stripe",
  "external_id": "cus_12345",
  "label": "Acme Corp Stripe customer",
  "metadata": { "plan": "enterprise" },
  "created_at": "2026-02-19T00:00:00Z",
  "updated_at": "2026-02-19T00:00:00Z"
}
```

---

---

## module.toml — Complete Template

Every SDK module declares a `module.toml` at its crate root. The SDK reads this at startup.

Source: `platform/platform-sdk/src/manifest/mod.rs`

```toml
# ── Required ─────────────────────────────────────────────────
[module]
name = "yourapp"                          # Module identifier
version = "1.0.0"                         # Cargo.toml version
description = "Your vertical application" # Human-readable

[server]
host = "0.0.0.0"
port = 8200                               # Pick an unused port

# ── Database (omit if stateless) ─────────────────────────────
[database]
migrations = "./db/migrations"            # Path relative to module.toml
auto_migrate = true                       # Run migrations on startup

# ── Event bus ────────────────────────────────────────────────
[bus]
type = "nats"                             # "nats" | "inmemory" | "none"

# ── Outbox publisher (SDK auto-spawns background publisher) ──
[events.publish]
outbox_table = "events_outbox"            # Must match your migration

# ── Auth / JWT (defaults work for most modules) ─────────────
[auth]
enabled = true                            # Default: true
jwks_url = "http://7d-auth-lb:8080/.well-known/jwks.json"  # Optional
refresh_interval = "5m"                   # JWKS refresh interval
fallback_to_env = true                    # Fall back to JWKS_URL env var

# ── Platform service dependencies ────────────────────────────
[platform.services]
party     = { enabled = true }                                    # PARTY_BASE_URL env var
inventory = { enabled = true, timeout_secs = 60 }                # INVENTORY_BASE_URL
bom       = { enabled = true, default_url = "http://localhost:8107" }  # Fallback URL

# ── SDK compatibility ────────────────────────────────────────
[sdk]
min_version = "0.1.0"                     # Minimum SDK version required
```

**Supported sections:** `[module]` (required), `[server]`, `[database]`, `[bus]`, `[events.publish]`, `[auth]`, `[cors]`, `[health]`, `[rate_limit]`, `[platform.services]`, `[sdk]`.

**Minimal viable module.toml** (stateless service, no events):
```toml
[module]
name = "yourapp"
version = "1.0.0"

[server]
port = 8200

[sdk]
min_version = "0.1.0"
```

**Typical vertical app module.toml** (DB + NATS + outbox):
```toml
[module]
name = "yourapp"
version = "1.0.0"
description = "Your vertical application"

[server]
host = "0.0.0.0"
port = 8200

[database]
migrations = "./db/migrations"
auto_migrate = true

[bus]
type = "nats"

[events.publish]
outbox_table = "events_outbox"

[sdk]
min_version = "0.1.0"
```

---

## Gotchas

Things that have tripped up agents and developers.

### event_type IS the NATS subject

The SDK publisher uses the `event_type` column from the outbox directly as the NATS subject. No prefix is added. If you store `"order.created"` in event_type, the NATS subject will be `"order.created"` — not `"yourapp.events.order.created"`. Store the full subject: `"yourapp.order_created"`.

### The SDK publisher replaces per-module publishers

Older modules had hand-written publisher tasks that formatted subjects with `format!("{module}.events.{event_type}")`. The SDK publisher does NOT do this — it uses event_type as-is. If you copy old AR publisher code instead of using the SDK, your subjects will be wrong.

### outbox_table must match your migration

The table name in `[events.publish].outbox_table` must exactly match the table name in your SQL migration. If they differ, the SDK publisher silently publishes nothing.

### enqueue_event_tx vs enqueue_event

`enqueue_event()` (no `_tx` suffix) is deprecated. It does NOT run in the same transaction as your domain mutation. Use `enqueue_event_tx()` which takes a `&mut Transaction` — this guarantees atomicity.

### Consumer idempotency is your responsibility

The SDK publishes events but does not deduplicate on the consumer side. Your consumer must check `processed_events` by `event_id` and skip duplicates. NATS JetStream delivers at-least-once.

### Missing [events.publish] = no publisher

If you forget the `[events.publish]` section in module.toml, the SDK will NOT start the outbox publisher. Events pile up in the outbox table unpublished. Check with: `SELECT COUNT(*) FROM events_outbox WHERE published_at IS NULL;`

### [platform.services] env vars

Each entry in `[platform.services]` expects an env var: `{SERVICE_NAME}_BASE_URL` (uppercase, hyphens become underscores). Example: `shipping-receiving` → `SHIPPING_RECEIVING_BASE_URL`. If the env var is missing and no `default_url` is set, startup fails.

---

> See `docs/PLATFORM-CONSUMER-GUIDE.md` for the master index and critical concepts.
