# Payments Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Proven Module (v1.1.0)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | SageDesert | Initial vision doc — documented existing v1.1.0 module from source: business problem, design principles, data ownership, state machines, events, integration points, invariants, API surface, decision log. |

---

## The Business Problem

Every multi-tenant SaaS platform that bills customers eventually faces the same challenge: **collecting money reliably without losing track of what happened.**

A tenant sends an invoice. The customer needs to pay it. Between "invoice sent" and "money collected" lies a minefield: payment processors timeout, webhooks arrive out of order, cards get declined, and retries need to happen on a schedule without double-charging anyone. The platform needs to know — with certainty — whether a payment succeeded, failed, or is stuck in limbo.

Small platforms solve this with a Stripe checkout link and a prayer. That works until a webhook gets dropped, a retry fires while a previous attempt is still in-flight, or a customer disputes a charge that was already retried. At scale, the cost of "probably succeeded" is chargebacks, angry customers, and reconciliation nightmares.

The Payments module exists to make payment collection **deterministic** — every attempt is tracked, every outcome is recorded, every retry follows a fixed schedule, and ambiguous results are quarantined until resolved.

---

## What the Module Does

The Payments module is the **authoritative system for payment execution** across all tenant applications on the 7D Solutions Platform. It owns the relationship with the payment service provider (PSP) and ensures no other module or application calls the PSP directly.

It answers four questions:
1. **Did the payment go through?** — A deterministic attempt ledger tracks every payment attempt with exactly-once enforcement.
2. **What happens when it fails?** — Fixed retry windows (day 0, +3, +7) with automatic scheduling and terminal failure after exhaustion.
3. **What if we don't know?** — The UNKNOWN protocol quarantines ambiguous results, blocks retries, and reconciles via PSP polling.
4. **How does the customer pay?** — Checkout sessions provide a Tilled.js-compatible flow where product apps never touch PSP credentials.

---

## Who Uses This

The module is a platform service. It does not have its own frontend — product applications consume its API and events.

### Product Applications (e.g., TrashTech)
- Create checkout sessions for customer-facing payment pages
- Receive `client_secret` for Tilled.js browser-side payment completion
- Poll session status to update their own UI
- Never handle PSP credentials or call PSP APIs directly

### AR Module (Accounts Receivable)
- Emits `ar.payment.collection.requested` when an invoice is due for collection
- Receives `payments.payment.succeeded` / `payments.payment.failed` events
- Never calls Payments directly — all communication is event-driven

### Operations / Finance
- Reviews payment attempt history per invoice
- Monitors UNKNOWN state durations via Prometheus metrics
- Uses admin endpoints for projection status and consistency checks

### System (Event Consumer + Retry Scheduler)
- Consumes AR payment collection events with idempotent processing
- Schedules retries at fixed windows based on first attempt date
- Reconciles UNKNOWN attempts by polling the PSP

---

## Design Principles

### PSP Abstraction — Product Apps Never Touch the PSP
The Payments module owns all PSP integration. Product apps create checkout sessions and receive client secrets. They never see API keys, never call PSP endpoints, never handle webhook signatures. This keeps PCI scope to a single module and allows PSP switching without touching downstream apps.

### Exactly-Once Side Effects
Every payment attempt is gated by a UNIQUE constraint on `(app_id, payment_id, attempt_no)`. Deterministic idempotency keys are sent to the PSP. Webhook processing uses `SELECT FOR UPDATE` locking with `webhook_event_id` deduplication. The result: a payment cannot be charged twice for the same attempt, and duplicate webhooks are no-ops.

### UNKNOWN Is a First-Class State
When a PSP response is ambiguous (timeout, error, unexpected status), the attempt enters UNKNOWN state rather than being forced into success or failure. UNKNOWN blocks retries (customer is not at fault) and blocks subscription suspension. Reconciliation polls the PSP to resolve UNKNOWN to a terminal state. This prevents double-charging and premature account actions.

### Fixed Retry Windows — No Configurability
Retry windows are hardcoded at +0, +3, +7 days from the first attempt. This is intentional: deterministic scheduling means the system behaves identically in every environment. Exactly one attempt per window, enforced by attempt number derivation from window index.

### Event-Driven Integration — No Cross-Module Calls
Payments consumes events from AR and emits events that AR, GL, and Notifications can subscribe to. It never calls another module's API at runtime. It never queries another module's database. This means Payments boots and functions independently.

### Guard → Mutate → Emit
Every state transition follows the same pattern: lifecycle guards validate the transition (zero side effects), the mutation executes, and events are emitted atomically within the same database transaction. Guards are pure validation — no I/O, no HTTP calls, no event emission.

---

## Current Scope (v1.1.0)

### Built and Proven
- Payment attempt ledger with exactly-once enforcement (UNIQUE constraint)
- Payment attempt state machine: attempting → succeeded / failed_retry / failed_final / unknown
- Lifecycle functions with guard → mutate → emit pattern
- UNKNOWN protocol: quarantine, retry blocking, PSP reconciliation
- Fixed retry windows (+0, +3, +7 days) with deterministic scheduling
- Tilled PSP integration: PaymentIntent create, confirm, status query
- Mock PSP for development and testing
- Webhook signature verification: HMAC-SHA256 with replay window (±5 min)
- Webhook secret rotation overlap (two secrets accepted simultaneously)
- Checkout sessions: create, get, present, status poll
- Checkout session state machine: created → presented → completed / failed / canceled / expired
- Tilled webhook handler: idempotent session status updates
- Event consumption: `ar.payment.collection.requested` with idempotent processing
- Event emission: `payment.succeeded`, `payment.failed` via outbox
- Outbox publisher: background task, 1-second poll interval
- Dead Letter Queue for events that fail after all retries
- Envelope validation (platform EventEnvelope contract)
- Deterministic idempotency key generation (`payment:attempt:{app_id}:{payment_id}:{attempt_no}`)
- Invariant assertion functions (duplicate detection, UNKNOWN compliance, terminal immutability)
- Prometheus metrics: attempt counts, UNKNOWN duration, retry counts, HTTP SLOs, consumer lag
- Admin endpoints: projection status, consistency check, projection list
- Health, readiness, version endpoints
- URL validation: HTTPS-only redirect URLs, no injection characters

### Explicitly Out of Scope
- Multi-PSP routing (only Tilled supported; mock for dev)
- Refunds and disputes
- Stored payment methods / customer vault
- Subscription billing (recurring charge scheduling)
- Currency conversion
- PCI-DSS Level 1 compliance (no raw card data stored — Tilled.js handles this)
- Partial payments or split payments
- Payment method management UI
- Scheduled session expiry (future: time-based created → expired transition)

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum | Port 8088 (default) |
| Database | PostgreSQL | Dedicated database, SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate; InMemory bus for dev |
| Auth | JWT via platform `security` crate | Optional claims middleware |
| PSP | Tilled | PaymentIntent flow via `api.tilled.com` |
| Webhook verification | HMAC-SHA256 | `tilled-signature` header, ±5 min replay window |
| Outbox | Platform outbox pattern | `payments_events_outbox` table |
| Metrics | Prometheus | `prometheus-client` + `prometheus` crates |
| Projections | Platform `projections` crate | Admin endpoints for status/consistency |
| Crate | `payments-rs` | Single crate, v1.1.0 |

---

## Structural Decisions (The "Walls")

### 1. PSP is an implementation detail — the module owns the abstraction
Product apps interact with checkout sessions and events. They never know or care whether the PSP is Tilled, Stripe, or a mock. The processor selection (`PAYMENTS_PROVIDER`) is a runtime config. Switching PSPs requires only implementing a new processor and updating config — no downstream changes.

### 2. Payment attempts are immutable ledger entries
Each attempt is a row in `payment_attempts` with a UNIQUE constraint on `(app_id, payment_id, attempt_no)`. Status transitions go through lifecycle guards. Once an attempt reaches a terminal state (succeeded, failed_final), no further transitions are allowed. The ledger is append-only at the attempt grain.

### 3. UNKNOWN is quarantined, not guessed
When the PSP response is ambiguous, the system records UNKNOWN rather than assuming success or failure. UNKNOWN blocks all downstream actions (retries, suspension) until reconciliation resolves it. This is more expensive operationally (requires PSP polling) but eliminates the worst-case scenario: double-charging a customer or prematurely suspending their account.

### 4. Retry windows are hardcoded, not configurable
Three windows: day 0, +3, +7. No per-tenant override, no backoff curves, no configuration. This eliminates an entire class of bugs (misconfigured retry schedules) and makes retry behavior identical across all environments. The windows match AR module retry windows for cross-module consistency.

### 5. Webhook signatures are verified before any database write
The mutation order is enforced: signature validation → envelope validation → attempt ledger gating → lifecycle mutation → event emission. No database writes occur before signature verification. This prevents replay attacks from causing any state change.

### 6. Checkout sessions and payment attempts are separate concerns
Checkout sessions track the customer-facing Tilled.js flow (created → presented → completed). Payment attempts track the AR-driven collection flow (attempting → succeeded/failed). They share the same PSP (Tilled) but have independent state machines and tables. A checkout session can exist without a payment attempt (customer pays via hosted page) and a payment attempt can exist without a checkout session (AR-triggered server-side collection).

### 7. Tenant isolation via app_id / tenant_id on every table
Standard platform multi-tenant pattern. Payment attempts use `app_id` as the tenant discriminator. Checkout sessions use `tenant_id`. Every query filters by the appropriate tenant field.

### 8. No mocking in integration tests
Integration tests hit real Postgres. Webhook signature tests use real HMAC computation. The mock processor is a development-time convenience, not a test double for the database layer.

---

## Domain Authority

Payments is the **source of truth** for:

| Domain Entity | Payments Authority |
|---------------|-------------------|
| **Payment Attempts** | Deterministic ledger: app_id, payment_id, attempt_no, status, PSP reference, idempotency key, webhook correlation. UNIQUE constraint enforces exactly-once. |
| **Checkout Sessions** | Customer-facing payment flow: session ID, Tilled PaymentIntent ID, client secret, status lifecycle (created → presented → completed/failed/canceled/expired). |
| **Webhook Events** | Verified PSP callbacks: signature validation, replay window enforcement, idempotent processing via `webhook_event_id`. |
| **PSP Integration** | Tilled API credentials, PaymentIntent creation/confirmation/query, webhook secret rotation. |
| **Retry Scheduling** | Fixed window discipline: which attempts are eligible, which window is active, when retries are blocked (UNKNOWN protocol). |
| **Reconciliation** | UNKNOWN → terminal state resolution via PSP polling with bounded retry. |

Payments is **NOT** authoritative for:
- Invoice state, due dates, or line items (AR module owns this)
- Customer master data or billing addresses (AR module owns this)
- GL journal entries or account balances (GL module owns this)
- Subscription lifecycle or renewal schedules (Subscriptions module owns this)
- Notification delivery or escalation rules (Notifications module owns this)

---

## Data Ownership

### Tables Owned by Payments

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **payment_attempts** | Deterministic attempt ledger | `id` (UUID), `app_id`, `payment_id` (UUID), `invoice_id`, `attempt_no` (0-2), `status` (attempting\|succeeded\|failed_retry\|failed_final\|unknown), `attempted_at`, `completed_at`, `processor_payment_id`, `payment_method_ref`, `failure_code`, `failure_message`, `webhook_event_id`, `idempotency_key`, `correlation_id` |
| **checkout_sessions** | Customer-facing payment flow | `id` (UUID), `invoice_id`, `tenant_id`, `amount_minor`, `currency`, `processor_payment_id`, `client_secret`, `return_url`, `cancel_url`, `status` (created\|presented\|completed\|failed\|canceled\|expired), `presented_at` |
| **payments_events_outbox** | Transactional outbox for reliable event publishing | `id`, `event_id` (UUID), `event_type`, `occurred_at`, `tenant_id`, `correlation_id`, `causation_id`, `payload` (JSONB), `published_at`, full envelope metadata (source_module, source_version, schema_version, replay_safe, trace_id, mutation_class, etc.) |
| **payments_processed_events** | Idempotent event consumption tracking | `id`, `event_id` (UUID), `event_type`, `source_module`, `processed_at` |
| **failed_events** | Dead Letter Queue for events that fail after all retries | `id`, `event_id` (UUID), `subject`, `tenant_id`, `envelope_json` (JSONB), `error`, `retry_count` |

**Monetary Precision:** All monetary amounts use **integer minor units** (`amount_minor` in cents). Currency stored as ISO 4217 code.

**Key Constraints:**
- `UNIQUE (app_id, payment_id, attempt_no)` on `payment_attempts` — exactly-once enforcement
- `CHECK (status IN (...))` on `checkout_sessions` — valid status values only
- `UNIQUE (event_id)` on outbox and processed events — no duplicate events

### Data NOT Owned by Payments

Payments **MUST NOT** store:
- Invoice details, line items, or due dates (AR owns invoices)
- Customer billing addresses or contact information (AR owns customers)
- Raw card numbers, CVVs, or PAN data (Tilled.js handles PCI scope)
- GL account codes or journal entries (GL owns the ledger)
- Subscription plans, renewal dates, or suspension state (Subscriptions owns this)

---

## Payment Attempt State Machine

```
ATTEMPTING ──→ SUCCEEDED (terminal)
    |
    ├──→ FAILED_RETRY ──→ ATTEMPTING (retry window)
    |
    ├──→ FAILED_FINAL (terminal)
    |
    └──→ UNKNOWN ──→ reconciliation ──→ SUCCEEDED / FAILED_RETRY / FAILED_FINAL
```

### Transition Rules

| From | Allowed To | Guard |
|------|-----------|-------|
| attempting | succeeded, failed_retry, failed_final, unknown | — |
| failed_retry | attempting | Retry window must be active; new attempt_no required |
| unknown | succeeded, failed_retry, failed_final | Reconciliation resolves via PSP poll |
| succeeded | *(terminal)* | No further transitions |
| failed_final | *(terminal)* | No further transitions |

### UNKNOWN Protocol

| Rule | Enforcement |
|------|-------------|
| UNKNOWN blocks retry scheduling | `get_payments_for_retry()` excludes status='unknown' |
| UNKNOWN blocks subscription suspension | Downstream consumers must check for UNKNOWN |
| Reconciliation resolves UNKNOWN | `reconcile_unknown_attempt()` polls PSP, transitions to terminal |
| Bounded PSP polling | Max 3 attempts with exponential backoff (1s, 2s, 4s) |
| StillUnknown defers resolution | If PSP still doesn't know, UNKNOWN state is preserved |

---

## Checkout Session State Machine

```
created ──→ presented ──→ completed (terminal)
    |            |
    |            ├──→ failed (terminal)
    |            |
    |            └──→ canceled (terminal)
    |
    ├──→ completed (webhook: page never visited)
    ├──→ failed (webhook)
    ├──→ canceled (webhook)
    └──→ expired (future: scheduled expiry)
```

### Transition Rules

| From | Allowed To | Trigger |
|------|-----------|---------|
| created | presented | POST .../present (hosted page load, idempotent) |
| created | completed, failed, canceled | Tilled webhook (page never visited) |
| presented | completed, failed, canceled | Tilled webhook |
| completed, failed, canceled, expired | *(terminal)* | No further transitions |

### Idempotency

- Webhook handler only updates sessions in `created` or `presented` state
- Terminal sessions (completed/failed/canceled/expired) are never overwritten (UPDATE matches 0 rows)
- Present endpoint is idempotent: calling it on an already-presented session is a no-op

---

## Events Produced

All events use the platform `EventEnvelope` and are written to the module outbox atomically with the triggering mutation.

| Event | Trigger | Key Payload Fields |
|-------|---------|-------------------|
| `payments.events.payment.succeeded` | Payment attempt transitions to succeeded | `payment_id`, `invoice_id`, `ar_customer_id`, `amount_minor`, `currency`, `processor_payment_id`, `payment_method_ref` |
| `payments.events.payment.failed` | Payment attempt transitions to failed | `payment_id`, `invoice_id`, `ar_customer_id`, `amount_minor`, `currency`, `failure_code`, `failure_message`, `processor_payment_id`, `payment_method_ref` |

---

## Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| `ar.events.payment.collection.requested` | AR module | Creates payment attempt (attempt_no=0), processes via PSP, emits succeeded/failed event |

**Consumption Guarantees:**
- Idempotent: processed event IDs tracked in `payments_processed_events`
- Retry with backoff: 3 attempts with exponential backoff before DLQ
- Envelope validation: platform EventEnvelope fields validated before processing
- Cross-module envelope compatibility: accepts both `payload`/`data` and `source_module`/`producer` field names

---

## Integration Points

### AR (Event-Driven, Bidirectional)

AR emits `ar.payment.collection.requested` when an invoice is due for collection. Payments processes the payment and emits `payment.succeeded` or `payment.failed`. AR subscribes to these events to update invoice status. **No HTTP calls between modules** — all communication is via NATS events.

### GL (Event-Driven, One-Way)

`payment.succeeded` carries `amount_minor`, `currency`, and `invoice_id`. A GL consumer (not part of the Payments module) subscribes and posts journal entries. **Payments never calls GL.**

### Notifications (Event-Driven, One-Way)

Notifications can subscribe to `payment.succeeded` and `payment.failed` to send payment confirmations and failure alerts. **Payments never calls Notifications.**

### Tilled PSP (HTTP, Runtime)

The only external HTTP dependency at runtime. Payments calls Tilled to:
- Create PaymentIntents (checkout session flow)
- Confirm PaymentIntents (server-side collection flow)
- Query PaymentIntent status (UNKNOWN reconciliation, live status polling)

Tilled calls Payments via webhook:
- `payment_intent.succeeded` / `payment_intent.payment_failed` / `payment_intent.canceled`
- Signature verified via HMAC-SHA256 before any processing

### Product Applications (HTTP, Checkout Sessions)

Product apps call Payments HTTP endpoints to create and poll checkout sessions. They receive `client_secret` for Tilled.js integration. **Product apps never call Tilled directly.**

---

## Invariants

1. **Exactly-once attempt enforcement.** UNIQUE constraint on `(app_id, payment_id, attempt_no)` prevents duplicate attempts. Idempotency keys sent to PSP provide PSP-level deduplication.
2. **State machine transitions are guarded.** No direct SQL status updates — all transitions go through lifecycle guard functions that validate the from→to pair.
3. **Outbox atomicity.** Event emission and state mutation happen in the same database transaction. No orphaned state without events.
4. **Signature before state.** Webhook signature verification completes before any database write. Replay attacks cause zero state changes.
5. **UNKNOWN blocks retries.** Payments with status='unknown' are excluded from retry scheduling. Reconciliation must resolve UNKNOWN before any retry can proceed.
6. **Terminal states are immutable.** Succeeded and failed_final have no outgoing transitions. Webhook replays against terminal attempts update 0 rows.
7. **Retry window discipline.** Exactly one attempt per window (0, 1, 2). Max 3 attempts total. Window calculation is deterministic from first attempt date.
8. **Checkout session idempotency.** Webhook handler only transitions non-terminal sessions. Present endpoint is idempotent. Duplicate webhooks are no-ops.
9. **No cross-module database access.** Payments never queries AR, GL, or any other module's database. Retry scheduling uses `attempted_at` from its own `payment_attempts` table, not AR invoice due dates.
10. **Event consumption is idempotent.** Processed event IDs tracked in `payments_processed_events`. Duplicate event delivery is a no-op.

---

## API Surface (Summary)

### Checkout Sessions
- `POST /api/payments/checkout-sessions` — Create checkout session (returns client_secret for Tilled.js)
- `GET /api/payments/checkout-sessions/{id}` — Get session detail (includes client_secret, polls Tilled for live status)
- `POST /api/payments/checkout-sessions/{id}/present` — Mark session as presented (idempotent, created→presented)
- `GET /api/payments/checkout-sessions/{id}/status` — Lightweight status poll (no client_secret exposed)

### Webhooks
- `POST /api/payments/webhook/tilled` — Tilled PSP webhook callback (HMAC-SHA256 verified)

### Admin
- `POST /api/payments/admin/projection-status` — Query projection status (requires X-Admin-Token)
- `POST /api/payments/admin/consistency-check` — Run consistency check (requires X-Admin-Token)
- `GET /api/payments/admin/projections` — List projections (requires X-Admin-Token)

### Operational
- `GET /healthz` — Legacy health check
- `GET /api/health` — Health check (service name + version)
- `GET /api/ready` — Readiness probe (verifies DB connectivity)
- `GET /api/version` — Module identity and schema version
- `GET /metrics` — Prometheus metrics

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`

---

## Decision Log

Every significant product, architecture, or standards decision is recorded here. Do not re-open a decision without adding a new row that supersedes the old one.

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-15 | Payment attempt ledger with UNIQUE(app_id, payment_id, attempt_no) | Exactly-once enforcement at the database level eliminates duplicate charge risk regardless of application-level bugs | Platform Orchestrator |
| 2026-02-15 | UNKNOWN as a first-class status that blocks retries | Ambiguous PSP results must not trigger retries (double-charge risk) or account suspension (customer not at fault); reconciliation workflow resolves deterministically | Platform Orchestrator + ChatGPT |
| 2026-02-15 | Fixed retry windows (0, +3, +7 days) with no configurability | Deterministic scheduling eliminates misconfiguration bugs; matches AR retry windows for cross-module consistency | Platform Orchestrator |
| 2026-02-15 | Lifecycle guards validate ONLY — zero side effects | Separating validation from mutation prevents partial state changes; guards can be tested in isolation without mocking I/O | Platform Orchestrator + ChatGPT |
| 2026-02-15 | Signature verification before any database write | Prevents replay attacks from causing state changes; webhook body is untrusted until signature is verified | Platform Orchestrator |
| 2026-02-15 | Webhook secret rotation overlap — two secrets accepted simultaneously | Zero-downtime key rotation: deploy new secret, both accepted during overlap window, then remove old secret | Platform Orchestrator |
| 2026-02-21 | Checkout sessions as a separate concern from payment attempts | Customer-facing Tilled.js flow has different lifecycle than AR-driven server-side collection; separate state machines prevent coupling | Platform Orchestrator |
| 2026-02-22 | Expanded checkout session state machine (created→presented→completed\|failed\|canceled\|expired) | Hosted pay portal needs visibility into whether the page was loaded (presented) vs just created; status polling avoids exposing client_secret | Platform Orchestrator |
| 2026-02-22 | Webhook handler transitions only non-terminal sessions | Idempotent: terminal sessions (completed/failed/canceled) are never overwritten; UPDATE with status guard matches 0 rows for replays | Platform Orchestrator |
| 2026-02-22 | Redirect URLs must be absolute HTTPS with no control characters | Prevents open redirect attacks and injection via return_url/cancel_url parameters | Platform Orchestrator |
| 2026-02-22 | No cross-module dependency for retry scheduling | Retry anchor uses `attempted_at` from payment_attempts (attempt_no=0) instead of joining to AR invoice due dates; module isolation preserved | Platform Orchestrator |
