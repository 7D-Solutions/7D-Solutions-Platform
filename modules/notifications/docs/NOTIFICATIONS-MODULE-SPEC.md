# Notifications Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.0)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | MaroonHarbor (implementation agent) | Initial vision doc — reverse-engineered from source, migrations, tests, and OpenAPI contract |

---

## The Business Problem

Every multi-module platform that sends transactional messages — invoice reminders, payment confirmations, low-stock alerts, period-close warnings — faces the same problem: **notifications are scattered across the modules that trigger them.**

When the AR module handles an issued invoice, someone has to send the "due soon" email. When a payment fails, someone has to alert the customer. When inventory drops below the reorder point, someone has to notify the warehouse manager. If each module implements its own email/SMS/webhook delivery, you get duplicated retry logic, inconsistent templates, no central audit trail, and no way for a customer to manage their notification preferences in one place.

The alternative — a central notifications service that all modules delegate to — keeps delivery concerns in one place, ensures consistent retry and dead-letter behavior, and gives operations a single pane of glass for notification health.

---

## What the Module Does

The Notifications module is the **platform's outbound communication hub**. It subscribes to domain events from other modules, transforms them into scheduled or immediate notifications, and delivers them through configured channels.

It answers four questions:
1. **What happened?** — Listens to events from AR, Payments, Inventory, and GL to detect notification-worthy moments.
2. **Who should be told?** — Routes notifications to the right recipient via `recipient_ref` (tenant:entity composite key).
3. **When should they be told?** — Some notifications fire immediately (payment succeeded); others are scheduled (invoice due in 3 days, payment retry in 24 hours).
4. **Did delivery succeed?** — Tracks delivery status with retry, back-off, and dead-letter queue for exhausted retries.

---

## Who Uses This

The module is a platform service. It has no user-facing frontend — it consumes events and delivers outbound communications.

### Other Platform Modules (Event Producers)
- AR emits `ar.events.invoice.issued` — Notifications schedules a "due soon" reminder 3 days before the due date.
- Payments emits `payments.events.payment.succeeded` — Notifications sends an immediate payment confirmation.
- Payments emits `payments.events.payment.failed` — Notifications schedules a retry reminder 24 hours later.
- Inventory emits `inventory.low_stock_triggered` — Notifications creates a low-stock alert.
- GL provides `close_calendar` + `accounting_periods` tables — Notifications evaluates upcoming/overdue close deadlines and emits close-calendar reminders.

### Operations / Platform Admins
- Monitor notification health via `/metrics` (Prometheus) and admin endpoints.
- Inspect dead-letter queue for failed events.
- Check projection status and consistency via admin API.

### System (Background Dispatcher)
- Polls `scheduled_notifications` every 60 seconds (configurable).
- Claims due notifications via `FOR UPDATE SKIP LOCKED`.
- Delivers through the `NotificationSender` trait (currently `LoggingSender` stub).
- Retries with linear back-off (5-minute increments), max 5 attempts.
- Resets orphaned claims (stuck > 5 minutes) on each tick and at startup.

---

## Design Principles

### Event-Driven, Never Polled by Producers
Modules never call Notifications directly. They emit domain events; Notifications subscribes and reacts. This means the producing module has zero runtime dependency on Notifications. If Notifications is down, events queue in NATS and are consumed when it recovers.

### Idempotent at Every Layer
- **Event consumption:** The `processed_events` table gates every incoming event by `event_id`. Replayed events are silently skipped.
- **Outbox publishing:** Events are written to `events_outbox` atomically with the triggering mutation. The outbox publisher retries publishing without re-emitting.
- **Scheduled dispatch:** `FOR UPDATE SKIP LOCKED` prevents double-claiming. Orphaned claims are reset after 5 minutes.
- **Close calendar reminders:** Keyed by `(calendar_entry_id, reminder_key)` to prevent duplicate reminder emissions.

### Sender Abstraction — Channel-Agnostic
The `NotificationSender` trait decouples dispatch logic from delivery implementation. The current production implementation (`LoggingSender`) logs and succeeds. Replacing it with an email provider, SMS gateway, or webhook client requires implementing one method. Tests use `FailingSender` to exercise retry/backoff paths against real Postgres.

### Fail Loudly, Never Lose Events
Events that fail processing after retry are written to the `failed_events` dead-letter queue with the full original envelope, error message, and retry count. The DLQ write itself is logged as an error if it fails, ensuring no silent event loss.

### Standalone Operation
Notifications boots and runs with only Postgres and NATS (or InMemory bus for development). It does not require any other module to be running. Close-calendar evaluation requires access to GL's database tables, but this is a read-only cross-DB query that degrades gracefully if GL is unavailable.

---

## MVP Scope (v0.1.0)

### In Scope (Built)
- Event consumption from AR (`ar.events.invoice.issued`), Payments (`payments.events.payment.succeeded`, `payments.events.payment.failed`), and Inventory (`inventory.low_stock_triggered`)
- Idempotent event processing via `processed_events` table
- Scheduled notifications with `scheduled_notifications` table (pending, claimed, sent, failed states)
- Background dispatcher: 60-second poll, `FOR UPDATE SKIP LOCKED`, orphan reset, linear back-off retry (5 attempts)
- `NotificationSender` trait with `LoggingSender` (production stub) and `FailingSender` (test-only)
- Outbox pattern for all outgoing events (`events_outbox` with full envelope metadata)
- Dead-letter queue (`failed_events`) for exhausted retries
- Envelope validation at consumption boundary (event_id, occurred_at, tenant_id, source_module, payload)
- Close-calendar reminder evaluation: upcoming + overdue reminders with idempotency tracking
- Prometheus metrics: request latency, request count, consumer lag
- Admin endpoints: projection status, consistency check, projection list
- Health, readiness, and version endpoints
- OpenAPI contract (`contracts/notifications/notifications-v0.1.0.yaml`)
- Docker multi-stage build (cargo-chef for dependency caching)
- Retry with exponential backoff on event consumption (via `event_bus::consumer_retry`)

### Explicitly Out of Scope for v1
- Real delivery providers (email/SMS/webhook integrations) — currently `LoggingSender` stub
- Template engine (template rendering, variable substitution, versioned templates)
- Notification preferences (per-user channel opt-in/opt-out)
- Notification log / audit trail queryable by tenant
- Batch/digest mode (consolidate multiple notifications into one)
- Rate limiting per recipient
- Subscription management / unsubscribe links
- Push notifications (mobile/web)
- Webhook delivery with signature verification
- Frontend UI for notification management

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum 0.8 | Port 8089 (default) |
| Database | PostgreSQL | Dedicated database, SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate; InMemory bus for development |
| Auth | JWT via platform `security` crate | Optional claims middleware, rate limiting, timeout |
| Outbox | Platform outbox pattern | Same as all other modules |
| Projections | Platform `projections` crate | Admin projection status/consistency endpoints |
| Metrics | Prometheus | `/metrics` endpoint, SLO-oriented labels |
| Crate | `notifications-rs` | Single crate, library + binary layout |

---

## Structural Decisions (The "Walls")

### 1. Pure event consumer — no synchronous API calls from other modules
Other modules never call Notifications via HTTP to "send a notification." They emit domain events; Notifications subscribes. This means Notifications is invisible to producers — removing it changes nothing about AR or Payments behavior. Adding a new notification type is a subscription change in Notifications, not a code change in the producer.

### 2. Scheduled notifications are a first-class table, not the events outbox
`scheduled_notifications` is a separate table from `events_outbox`. The outbox handles reliable event publishing (Notifications → NATS). Scheduled notifications handle time-delayed delivery (e.g., "remind 3 days before due date"). These are different concerns with different lifecycle: outbox rows are short-lived (pending → published); scheduled rows may sit for days or weeks before becoming due.

### 3. NotificationSender trait as the delivery boundary
All actual delivery goes through a single `async fn send(&self, notif: &ScheduledNotification) -> Result<(), NotificationError>` method. This makes the dispatcher testable against real Postgres without needing email/SMS infrastructure. The trait is the only seam where a real provider plugs in.

### 4. Claim-based dispatch with FOR UPDATE SKIP LOCKED
The dispatcher doesn't just SELECT pending rows — it atomically claims them in a single UPDATE...RETURNING with `FOR UPDATE SKIP LOCKED`. This makes the dispatcher safe to run in multiple instances (horizontal scaling) without coordination. Orphaned claims (from crashes) are cleaned up on each tick.

### 5. Linear back-off with hard failure at 5 retries
Failed deliveries are rescheduled with `(retry_count + 1) * 5` minutes back-off. After 5 attempts, the row is marked `failed` and stays in the table for manual investigation. This prevents infinite retry loops while giving transient failures time to resolve.

### 6. Envelope validation at the consumption boundary
Every incoming event is validated for required fields (event_id, occurred_at, tenant_id, source_module, payload) before processing. Invalid envelopes are rejected with structured error messages. This catches contract violations early rather than allowing corrupt data into the pipeline.

### 7. Cross-module envelope compatibility
The consumer accepts both Payments-style envelopes (`source_module`, `correlation_id`, `payload`) and AR-style envelopes (`producer`, `trace_id`, `data`). This dual-format handling means Notifications works with both envelope conventions without requiring producer changes.

### 8. Close-calendar reminder idempotency via separate tracking table
Close-calendar reminders use `close_calendar_reminders_sent` (in the GL database) keyed by `(tenant_id, calendar_entry_id, reminder_key)`. The reminder_key encodes type and trigger date, so the same reminder is never emitted twice even if the evaluator runs repeatedly.

---

## Domain Authority

Notifications is the **source of truth** for:

| Domain Entity | Notifications Authority |
|---------------|------------------------|
| **Scheduled Notifications** | Time-delayed notification queue: recipient, channel, template key, payload, delivery schedule, status lifecycle (pending → claimed → sent/failed). |
| **Notification Delivery Status** | Whether a notification was sent, is pending retry, or has failed after exhausting retries. |
| **Outgoing Notification Events** | Events emitted by Notifications: delivery succeeded, low-stock alert created, close-calendar reminders. |
| **Event Processing State** | Which incoming events have been consumed (idempotency gate via processed_events). |
| **Dead Letter Queue** | Failed events with full envelope, error detail, and retry count for post-mortem investigation. |

Notifications is **NOT** authoritative for:
- Invoice amounts, due dates, or payment status (AR and Payments modules own this)
- Inventory stock levels or reorder points (Inventory module owns this)
- Accounting period close status or calendar configuration (GL module owns this)
- Customer contact information (future identity/CRM module would own this)
- Template content or rendering (future concern — currently template_key is an opaque string)

---

## Data Ownership

### Tables Owned by Notifications

All tables live in the Notifications database (`notifications_db`).

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **events_outbox** | Transactional outbox for reliable event publishing | `id` (BIGSERIAL), `event_id` (UUID, unique), `subject`, `payload` (JSONB), `tenant_id`, `status` (pending/published/failed), `retry_count`, `event_type`, `source_module`, `source_version`, `schema_version`, `occurred_at`, `replay_safe`, `trace_id`, `correlation_id`, `causation_id`, `mutation_class` |
| **processed_events** | Idempotency gate for consumed events | `event_id` (UUID, PK), `subject`, `tenant_id`, `source_module`, `processed_at` |
| **failed_events** | Dead-letter queue for events that failed all retries | `id` (BIGSERIAL), `event_id` (UUID, unique), `subject`, `tenant_id`, `envelope_json` (JSONB), `error`, `retry_count`, `failed_at` |
| **scheduled_notifications** | Time-delayed notification queue | `id` (UUID, PK), `recipient_ref`, `channel`, `template_key`, `payload_json` (JSONB), `deliver_at`, `status` (pending/claimed/sent/failed), `retry_count`, `last_attempt_at`, `created_at` |

### Tables Read by Notifications (Cross-DB, Read-Only)

| Table | Owner Module | Purpose in Notifications |
|-------|-------------|--------------------------|
| **close_calendar** | GL | Calendar entries with expected close dates, reminder offsets, overdue intervals |
| **accounting_periods** | GL | Period close status — Notifications skips already-closed periods |
| **close_calendar_reminders_sent** | GL | Idempotency tracking for close-calendar reminders (Notifications writes here) |

### Data NOT Owned by Notifications

Notifications **MUST NOT** store:
- Financial data (invoice amounts, payment amounts, account balances)
- Customer master data (names, email addresses, phone numbers)
- Inventory quantities or SKU data
- GL account codes or journal entries
- Authentication credentials or session data

---

## Scheduled Notification Lifecycle

```
pending ──→ claimed ──→ sent (terminal, success)
                │
                └──→ pending (retry, back-off) ──→ ... ──→ failed (terminal, 5 retries exhausted)
```

### Status Transitions

| From | To | Trigger |
|------|----|---------|
| pending | claimed | Dispatcher claims batch via `FOR UPDATE SKIP LOCKED` |
| claimed | sent | `NotificationSender.send()` succeeds |
| claimed | pending | `NotificationSender.send()` fails, retry_count < 5 (rescheduled with back-off) |
| claimed | failed | `NotificationSender.send()` fails, retry_count >= 5 |
| claimed | pending | Orphan reset (claimed but `last_attempt_at` > 5 minutes stale) |

---

## Events Produced

All events use the platform `EventEnvelope` and are written to the module outbox atomically.

| Event | Trigger | Key Payload Fields |
|-------|---------|-------------------|
| `notifications.delivery.succeeded` | Payment succeeded handler completes delivery simulation | `notification_id`, `channel`, `to`, `template_id`, `status`, `provider_message_id`, `attempts` |
| `notifications.low_stock.alert.created` | Low-stock event consumed from Inventory | `notification_id`, `channel`, `template_id`, `status` |
| `notifications.close_calendar.reminder` | Close-calendar evaluator detects upcoming/overdue period close | `calendar_entry_id`, `tenant_id`, `period_id`, `owner_role`, `reminder_type`, `expected_close_date`, `days_offset`, `message` |

---

## Events Consumed

| Event | Source Module | Action |
|-------|-------------|--------|
| `ar.events.invoice.issued` | AR | Schedules `invoice_due_soon` reminder 3 days before due date. Skipped if no `due_date` in payload. |
| `payments.events.payment.succeeded` | Payments | Simulates immediate delivery, emits `notifications.delivery.succeeded` to outbox. |
| `payments.events.payment.failed` | Payments | Schedules `payment_retry` reminder 24 hours from now. |
| `inventory.low_stock_triggered` | Inventory | Enqueues `notifications.low_stock.alert.created` to outbox. |

---

## Integration Points

### AR (Event Consumer)
Notifications subscribes to `ar.events.invoice.issued`. When an invoice is issued with a `due_date`, Notifications schedules an `invoice_due_soon` reminder 3 days before. The recipient is identified by `tenant_id:customer_id`. **AR never calls Notifications.**

### Payments (Event Consumer)
Notifications subscribes to `payments.events.payment.succeeded` and `payments.events.payment.failed`. Successful payments trigger an immediate delivery confirmation event. Failed payments schedule a retry reminder in 24 hours. **Payments never calls Notifications.**

### Inventory (Event Consumer)
Notifications subscribes to `inventory.low_stock_triggered`. When stock drops below the reorder point, Notifications enqueues a low-stock alert event. **Inventory never calls Notifications.**

### GL (Read-Only Cross-DB)
The close-calendar evaluator reads `close_calendar`, `accounting_periods`, and writes to `close_calendar_reminders_sent` in the GL database. This is the only cross-DB access in the module. **GL never calls Notifications; Notifications reads GL state on a schedule.**

### Platform Event Bus (NATS)
All event consumption and publishing goes through the platform `event-bus` crate. NATS is the production transport; `InMemoryBus` is available for local development. The outbox publisher polls at 100ms intervals and publishes pending events in batches of 100.

### Platform Security
JWT verification via the `security` crate with optional claims middleware. Rate limiting and timeout middleware are applied to all HTTP routes. Admin endpoints require `X-Admin-Token` header.

---

## Invariants

1. **Event idempotency is unbreakable.** Every consumed event is gated by `processed_events`. The same `event_id` is never handled twice.
2. **Outbox atomicity.** Every outgoing event is written to `events_outbox` in the same transaction as the triggering operation. No silent event loss.
3. **Scheduled notification claim safety.** `FOR UPDATE SKIP LOCKED` ensures no two dispatcher instances process the same notification.
4. **Orphan recovery.** Claimed notifications stuck longer than 5 minutes are automatically reset to pending on every dispatcher tick and at startup.
5. **Retry limit enforced.** After 5 failed delivery attempts, a notification is permanently marked `failed`. No infinite retry loops.
6. **DLQ write-through.** Events that fail all consumer retries are written to `failed_events` with the complete original envelope. If the DLQ write itself fails, an error-level log is emitted.
7. **Envelope validation at boundary.** Incoming events with missing or invalid envelope fields are rejected before processing. No corrupt data enters the pipeline.
8. **Close-calendar reminder idempotency.** Each reminder is keyed by `(calendar_entry_id, reminder_key)`. The same reminder cannot be emitted twice.

---

## API Surface (Summary)

Full OpenAPI contract: `contracts/notifications/notifications-v0.1.0.yaml`

### Health & Operational
- `GET /healthz` — Legacy health check
- `GET /api/health` — Service health (status, service name, version)
- `GET /api/ready` — Readiness probe (verifies DB connectivity, latency)
- `GET /api/version` — Module name, version, schema version
- `GET /metrics` — Prometheus metrics (SLO: latency, request count, consumer lag)

### Admin (Requires X-Admin-Token)
- `POST /api/notifications/admin/projection-status` — Query projection status
- `POST /api/notifications/admin/consistency-check` — Run consistency check
- `GET /api/notifications/admin/projections` — List all projections

### OpenAPI-Defined (Not Yet Implemented)
The OpenAPI contract defines endpoints for direct notification sending, status lookup, preference management, and template CRUD. These are aspirational — the current implementation is purely event-driven with no direct send API.

---

## Decision Log

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-12 | Event-driven consumption, no direct send API in v0.1 | Notifications should react to domain events, not be called synchronously. Keeps producing modules decoupled. Direct send API deferred to when real delivery providers are integrated. | Platform Orchestrator |
| 2026-02-12 | Separate `processed_events` table for idempotency | Platform-wide pattern. Every event consumer needs idempotency. Using the event_id as primary key prevents duplicate processing even under NATS at-least-once delivery. | Platform Orchestrator |
| 2026-02-12 | `failed_events` DLQ with full envelope preservation | Events must never be silently dropped. Storing the complete envelope enables manual replay after root-cause investigation. | Platform Orchestrator |
| 2026-02-16 | Full envelope metadata columns on events_outbox | Phase 16 constitutional envelope: event_type, source_module, source_version, schema_version, occurred_at, replay_safe, trace_id, correlation_id, causation_id, mutation_class. Enables distributed tracing and replay classification. | Platform Orchestrator |
| 2026-02-16 | Dual envelope format support (Payments vs AR style) | AR uses `producer`/`trace_id`/`data`; Payments uses `source_module`/`correlation_id`/`payload`. Rather than force all producers to align on one format, Notifications accepts both. Cheaper to handle here than to coordinate a breaking change across modules. | Platform Orchestrator |
| 2026-02-16 | mutation_class = SIDE_EFFECT for delivery events | Email/SMS delivery is a non-idempotent side effect. Marking it as SIDE_EFFECT in the envelope enables downstream replay filters to skip these events during recovery replay. | Platform Orchestrator |
| 2026-02-23 | Scheduled notifications in dedicated table, not outbox | Outbox rows are transient (pending → published in < 1 second). Scheduled notifications may wait days. Different lifecycle, different query patterns, different indexes. Separate table prevents outbox bloat and simplifies dispatcher queries. | Platform Orchestrator |
| 2026-02-23 | FOR UPDATE SKIP LOCKED for claim-based dispatch | Enables horizontal scaling of dispatchers without distributed locking. Multiple instances can safely process different batches concurrently. | Platform Orchestrator |
| 2026-02-23 | NotificationSender trait with LoggingSender stub | Decouples dispatcher logic from delivery infrastructure. Production can ship with logging-only delivery while the dispatch machinery is proven correct. Real providers plug in later without changing dispatcher code. | Platform Orchestrator |
| 2026-02-23 | Linear back-off: (retry_count + 1) * 5 minutes, max 5 retries | Simple, predictable. 5 → 10 → 15 → 20 → 25 minutes. Total window is ~75 minutes. Avoids exponential explosion while giving transient failures time to clear. | Platform Orchestrator |
| 2026-02-23 | Orphan reset: claimed rows stale > 5 minutes reset to pending | Handles process crashes mid-delivery. 5-minute window is long enough for any reasonable delivery attempt but short enough to avoid stuck notifications. Reset runs on every tick and at startup. | Platform Orchestrator |
| 2026-02-23 | invoice_due_soon reminder: 3 days before due date | Standard business practice — reminder early enough to act on but not so early it gets ignored. Configurable per-tenant reminders deferred to v2. | Platform Orchestrator |
| 2026-02-23 | payment_retry reminder: 24 hours after failure | Gives the customer time to resolve payment issues (update card, add funds) without being immediately nagged. | Platform Orchestrator |

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`
