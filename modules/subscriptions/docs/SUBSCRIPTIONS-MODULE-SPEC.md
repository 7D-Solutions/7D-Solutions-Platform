# Subscriptions Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.0)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | Platform Orchestrator | Initial vision doc — business problem, design principles, domain authority, data ownership, state machine, events, integration points, invariants, API surface, decision log. Documented from built source code, migrations, and contracts. |
| 1.1 | 2026-02-24 | Platform Orchestrator | Fresh-eyes review: fixed 10 inaccuracies. Marked 9 API routes as contract-only (not implemented). Removed 3 events that don't exist in code. Clarified paused/cancelled not in lifecycle guards. Noted cycle gating not wired into bill run. Fixed tenant isolation claim (bill_runs has no tenant_id). Noted DLQ table missing migration. Fixed event naming. |

---

## The Business Problem

Recurring revenue businesses — SaaS platforms, waste haulers with weekly pickups, property managers with monthly services — all share the same challenge: **billing cycles are invisible until an invoice is late or missing.**

A customer signs up for a monthly service. Someone has to remember to bill them every month, at the right amount, on the right date. If a payment fails, someone has to track the grace period and decide when to suspend service. If the business runs billing manually, invoices slip through. If they automate it naively, duplicate invoices get generated on retries.

The problem gets worse at scale. A business with 500 subscriptions can't manually track which ones billed this month and which ones didn't. A bill run that crashes midway through can't safely restart without risking double-billing. And the moment a customer disputes a duplicate charge, the business loses trust and money.

---

## What the Module Does

The Subscriptions module is the **authoritative system for recurring billing schedules and service agreements**. It owns the "when to bill" and "how much to bill" — but it never owns the invoice itself or the payment.

It answers four questions:
1. **What plans exist?** — Subscription plan templates with schedule, price, and currency.
2. **Who is subscribed to what?** — Active subscriptions linking an AR customer to a plan, with billing schedule and next bill date.
3. **When does billing happen?** — Bill runs that find due subscriptions and trigger invoice creation via the AR module's API.
4. **What happened when billing went wrong?** — Lifecycle state transitions (past due, suspended) driven by dunning events from AR.

Critically, Subscriptions **delegates invoice creation to AR** via HTTP API calls. It never stores invoice data, never stores payment references, and never emits financial truth events. AR is the single source of truth for invoices; Subscriptions is the single source of truth for billing schedules.

---

## Who Uses This

The module is a platform service consumed by any vertical application that manages recurring billing. It does not have its own frontend — it exposes an API that frontends consume.

### Business Administrator
- Creates subscription plans (monthly pickup service, weekly maintenance, etc.)
- Assigns subscriptions to customers by linking AR customer IDs to plans
- Triggers bill runs to generate invoices for due subscriptions
- Monitors bill run results (processed, created, failed counts)

### Operations / Finance
- Reviews subscription lifecycle events (activations, suspensions, cancellations)
- Tracks churn via subscription status changes
- Correlates billing cycles with AR invoice data for reconciliation

### System (Bill Run Scheduler)
- Finds subscriptions with `next_bill_date <= today` and `status = active`
- Creates invoices via AR API (direct HTTP calls — cycle gating module exists but is **not yet wired** into the bill run endpoint)
- Advances `next_bill_date` after successful invoice creation
- Records bill run outcomes for audit and idempotency
- **Note:** Current implementation queries all tenants — no `tenant_id` filter on the due subscriptions query

### System (Event Consumer)
- Consumer handler exists (`consumer.rs`) for `ar.invoice_suspended` events from AR dunning flow
- Applies suspension to matching subscriptions for that customer/tenant
- Uses `processed_events` table for idempotent event consumption
- **Note:** The consumer handler function exists but no NATS subscription loop is wired in `main.rs` — the handler must be invoked externally or the subscription loop is not yet implemented

---

## Design Principles

### Invoice Delegation, Not Ownership
Subscriptions never stores invoice data. When a bill run executes, it calls AR's API to create and finalize invoices. The response is used only to confirm success — no invoice fields are persisted in the Subscriptions database. This prevents data divergence between what Subscriptions thinks was billed and what AR actually recorded.

### Exactly-Once Invoice Per Cycle (Module Built, Not Yet Wired)
The cycle gating module (`cycle_gating.rs`, `gated_invoice_creation.rs`) implements advisory locks + UNIQUE constraint on `(tenant_id, subscription_id, cycle_key)` to prevent duplicate invoices. The pattern is: Gate → Lock → Check → Execute → Record. **However, the `execute_bill_run` endpoint currently calls AR directly without using the gating module.** Wiring the gating into the bill run is a remaining integration step.

### Guard → Mutation → Side Effect
All lifecycle transitions follow the same pattern: a pure guard function validates the transition (zero side effects), the database mutation occurs within a transaction, and the outbox event is written atomically in the same transaction. No orphaned state changes without corresponding events.

### Event-Driven Dunning Response
Subscriptions does not poll AR for payment status. Instead, AR emits `ar.invoice_suspended` when dunning reaches terminal escalation, and Subscriptions consumes this event to suspend the affected subscriptions. This keeps the boundary clean — AR owns dunning logic, Subscriptions owns subscription state.

### Standalone First
The module boots and runs without AR, Payments, GL, or Notifications. Bill runs will fail to create invoices if AR is down, but the subscription data, plans, and lifecycle state remain intact and operational.

---

## MVP Scope (v0.1.0)

### Implemented
- Subscription plans — database table and model exist; **no HTTP routes** (contract-only in OpenAPI)
- Subscriptions — database table and model exist; **no HTTP routes** for create/list/get/pause/resume/cancel (contract-only in OpenAPI)
- Bill run execution: find due subscriptions, call AR API directly, advance next bill date (`POST /api/bill-runs/execute`)
- Bill run idempotency via `bill_run_id` UNIQUE constraint
- Cycle gating module: advisory locks + UNIQUE constraint + attempt ledger (`cycle_gating.rs`, `gated_invoice_creation.rs`) — **exists as library code but NOT wired into the bill run endpoint**
- Subscription lifecycle state machine: active, past_due, suspended — guard-protected transitions with Guard → Mutation → Outbox atomicity
- Lifecycle guards for paused/cancelled — **NOT implemented** (DB CHECK constraint allows these statuses but `SubscriptionStatus` enum and `transition_guard()` only cover active/past_due/suspended)
- Event consumption handler: `ar.invoice_suspended` → subscription suspension (`consumer.rs`) — handler exists but no NATS subscription loop in `main.rs`
- Dead letter queue: `dlq.rs` module exists but **`failed_events` table has no migration** — function cannot execute
- Outbox publisher with infinite retry for at-least-once event delivery
- Prometheus metrics (cycles attempted/completed, churn, HTTP latency, consumer lag)
- Admin endpoints for projection status and consistency checks (3 endpoints, wired)
- OpenAPI contract (`contracts/subscriptions/subscriptions-v1.yaml`) — defines full API surface including unimplemented routes
- Events emitted: 2 events — `subscriptions.status.changed` (past_due/suspended transitions only), `billrun.completed`
- Envelope validation (event_id, occurred_at, tenant_id, source_module, source_version, payload)
- JWT-based auth with permission layer (`SUBSCRIPTIONS_MUTATE`) — wired in `main.rs`
- Invariant assertion functions for cycle gating integrity (`invariants.rs`)

### Explicitly Out of Scope for v1
- Usage-based billing (metered subscriptions)
- Proration logic (flag exists but disabled in MVP)
- Trial periods and introductory pricing
- Subscription upgrades/downgrades (plan changes mid-cycle)
- Multi-currency within a single subscription
- Automated bill run scheduling (currently trigger-based only)
- Notification integration (subscribers would consume events)
- Customer self-service (portal for managing subscriptions)
- Revenue recognition and deferred revenue tracking

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum | Port 8087 (default) |
| Database | PostgreSQL | Dedicated database, SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate, configurable (NATS or in-memory) |
| Auth | JWT via platform `security` crate | Tenant-scoped, permission-based (`SUBSCRIPTIONS_MUTATE`) |
| Outbox | Platform outbox pattern | Same as all other modules, with envelope metadata |
| Metrics | Prometheus | `/metrics` endpoint with SLO counters and histograms |
| Projections | Platform `projections` crate | Admin endpoints for projection status |
| Crate | `subscriptions-rs` | Single crate, standard module layout |

---

## Structural Decisions (The "Walls")

These are decisions that are cheap to make correctly now and very expensive to retrofit later.

### 1. Never store invoice data — delegate to AR
Subscriptions calls AR's API to create invoices and reads the response to confirm success. It never stores invoice IDs, amounts, or statuses in its own database (except temporarily in the `subscription_invoice_attempts` ledger for cycle gating). AR is the single source of truth for all invoice data. This eliminates an entire class of data consistency problems.

### 2. Cycle gating uses advisory locks + UNIQUE constraint (built, not wired)
Two layers of protection: `pg_advisory_xact_lock` prevents concurrent bill runs from processing the same subscription cycle simultaneously, and the UNIQUE constraint on `(tenant_id, subscription_id, cycle_key)` provides a database-level guarantee that no duplicate attempt record exists. The advisory lock is released before the expensive AR API calls to minimize contention. **Status:** The `cycle_gating.rs` and `gated_invoice_creation.rs` modules implement this pattern fully, but `execute_bill_run` in `routes.rs` calls AR directly without using the gating module.

### 3. Lifecycle transitions are guard-protected
All status changes go through `transition_guard()` — a pure function that validates the from→to pair and returns `Ok(())` or an error. The guard has zero side effects. Database mutations and event emissions happen only after the guard approves. This makes the state machine testable without a database.

### 4. Event consumption is idempotent
The `processed_events` table tracks which event IDs have already been handled. The `process_event_idempotent()` wrapper checks this table before processing and records the event ID after success. This means event replays, NATS redeliveries, and crash recovery all result in the same final state.

### 5. Outbox events carry full envelope metadata
Every outbox record includes envelope metadata (event_id, tenant_id, source_module, source_version, trace_id, correlation_id, causation_id, mutation_class). This makes events self-describing and supports distributed tracing, replay analysis, and operational queries without needing to deserialize the payload.

### 6. AR API calls happen outside the gating transaction (in gating module)
In the `gated_invoice_creation.rs` module, the gating transaction (acquire lock → check attempt → record attempt) commits before calling AR. This keeps the advisory lock duration under 50ms. If the AR call fails, the attempt is marked as failed in a separate transaction. This design trades "attempt recorded but AR not called" (recoverable) for "long lock hold blocking other subscriptions" (unrecoverable contention). **Note:** The `execute_bill_run` endpoint does not use this module — it calls AR directly within a simple loop.

### 7. Tenant isolation via tenant_id (partial)
Standard platform multi-tenant pattern. Domain tables (`subscription_plans`, `subscriptions`, `subscription_invoice_attempts`) have `tenant_id` as a non-nullable field with indexes. **Exception:** `bill_runs` has no `tenant_id` column — bill runs operate across all tenants. The `execute_bill_run` query for due subscriptions does not filter by `tenant_id`.

---

## Domain Authority

Subscriptions is the **source of truth** for:

| Domain Entity | Subscriptions Authority |
|---------------|------------------------|
| **Subscription Plans** | Plan templates: name, schedule (weekly/monthly/custom), price in minor units, currency, proration flag. |
| **Subscriptions** | Active agreements linking an AR customer to a plan: status, schedule, price, start date, next bill date, paused/cancelled timestamps. |
| **Bill Runs** | Billing cycle executions: bill_run_id (idempotency key), execution date, counts (processed, created, failed), status (running/completed/failed). |
| **Subscription Invoice Attempts** | Cycle gating ledger: tracks which subscription cycles have had invoice generation attempted, with status (attempting/succeeded/failed_retry/failed_final) and AR invoice ID on success. |
| **Billing Schedule** | When each subscription is next due for billing (`next_bill_date`), advanced after each successful invoice creation. |

Subscriptions is **NOT** authoritative for:
- Invoice data, amounts, line items, or finalization status (AR module owns this)
- Payment status, payment methods, or transaction records (Payments module owns this)
- GL account balances or journal entries (GL module owns this)
- Dunning rules, escalation thresholds, or grace periods (AR module owns this)
- Customer master data (AR module owns the customer record)

---

## Data Ownership

### Tables Owned by Subscriptions

Domain tables use `tenant_id` for multi-tenant isolation. Every query **MUST** filter by `tenant_id` (exception: `bill_runs` — see note below).

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **subscription_plans** | Plan templates | `id`, `tenant_id`, `name`, `description`, `schedule` (weekly\|monthly\|custom), `price_minor`, `currency`, `proration_enabled` |
| **subscriptions** | Active service agreements | `id`, `tenant_id`, `ar_customer_id`, `plan_id` (FK→subscription_plans), `status` (active\|past_due\|suspended\|paused\|cancelled), `schedule`, `price_minor`, `currency`, `start_date`, `next_bill_date`, `paused_at`, `cancelled_at` |
| **bill_runs** | Billing cycle execution records | `id`, `bill_run_id` (UNIQUE), `execution_date`, `subscriptions_processed`, `invoices_created`, `failures`, `status` (running\|completed\|failed) — **NOTE: no `tenant_id` column** |
| **subscription_invoice_attempts** | Cycle gating ledger | `id`, `tenant_id`, `subscription_id` (FK→subscriptions), `cycle_key` (YYYY-MM), `cycle_start`, `cycle_end`, `status` (attempting\|succeeded\|failed_retry\|failed_final), `ar_invoice_id`, `idempotency_key`, UNIQUE(tenant_id, subscription_id, cycle_key) |
| **events_outbox** | Standard platform outbox | Module-owned, same schema as other modules with envelope metadata columns |
| **processed_events** | Event deduplication | `event_id` (PK), `subject`, `processed_at` |

**Monetary Precision:** All monetary amounts use **integer minor units** (e.g., `price_minor` in cents). Currency stored as 3-letter ISO 4217 code.

### Data NOT Owned by Subscriptions

Subscriptions **MUST NOT** store:
- Invoice data (amounts, line items, status, finalization state)
- Payment references, transaction IDs, or payment method data
- GL account codes or journal entry details
- Customer contact information or billing addresses
- Dunning attempt counts or escalation state (AR tracks dunning)

---

## Subscription State Machine

### Guard-Protected Transitions (lifecycle module)

The `SubscriptionStatus` enum and `transition_guard()` function cover three statuses:

```
ACTIVE ──→ PAST_DUE ──→ SUSPENDED
  ↑    └───────────────────┘  │
  └───────────────────────────┘
```

| From | Allowed To | Guard | Trigger |
|------|-----------|-------|---------|
| active | past_due | `transition_guard()` | Payment failure (dunning event) |
| active | suspended | `transition_guard()` | Terminal dunning escalation |
| past_due | active | `transition_guard()` | Payment recovered |
| past_due | suspended | `transition_guard()` | Grace period expired |
| suspended | active | `transition_guard()` | Reactivation (payment recovered) |

**Idempotent transitions:** Same-state transitions (active→active, past_due→past_due, suspended→suspended) are explicitly allowed — processing the same event twice produces the same result without error.

**Illegal transitions:** `suspended → past_due` (cannot go backwards in escalation).

**Event emission asymmetry:** `transition_to_past_due` and `transition_to_suspended` write `subscriptions.status.changed` events to the outbox atomically within the same transaction. `transition_to_active` does **NOT** emit events (comment in code: "Future").

### Database-Level States (not guard-protected)

The DB CHECK constraint allows two additional statuses: `paused` and `cancelled`. These exist in the database schema and the `Subscription` model struct but are **NOT** represented in the `SubscriptionStatus` enum and have **no guard functions or transition logic**. No HTTP routes exist for pause/resume/cancel operations.

```
paused    — DB state only, no lifecycle guard
cancelled — DB state only, no lifecycle guard (intended terminal)
```

---

## Events Produced

All events use the platform `EventEnvelope` and are written to the module outbox atomically with the triggering mutation. The outbox publisher (`publisher.rs`) publishes to NATS with subject `subscriptions.events.<outbox_subject>`.

| Event (outbox subject) | NATS Subject | Trigger | Key Payload Fields |
|-------|---------|---------|-------------------|
| `subscriptions.status.changed` | `subscriptions.events.subscriptions.status.changed` | Lifecycle transition to past_due or suspended | `subscription_id`, `tenant_id`, `from_status`, `to_status`, `reason` |
| `billrun.completed` | `subscriptions.events.billrun.completed` | Bill run finished | `bill_run_id`, `subscriptions_processed`, `invoices_created`, `failures`, `execution_time` |

**Events NOT yet emitted (models exist but no emission code):**
- `subscriptions.created` — no subscription creation route exists
- `subscriptions.paused` — no pause route exists
- `subscriptions.resumed` — no resume route exists
- `subscriptions.status.changed` for reactivation (active) — `transition_to_active` does not write to outbox

---

## Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| `ar.invoice_suspended` | AR (dunning) | Suspends all active/past_due subscriptions for the affected customer+tenant. Idempotent via `processed_events`. |

---

## Integration Points

### AR (HTTP Command, Required for Billing)

Bill runs call AR's API to create and finalize invoices:
- `POST /api/ar/invoices` — Create invoice for a subscription cycle
- `POST /api/ar/invoices/{id}/finalize` — Finalize the created invoice

**Failure mode:** If AR is unavailable, the bill run increments its failure counter for that subscription and continues processing others. The subscription's `next_bill_date` is NOT advanced (it will be retried on the next bill run). If cycle gating were wired in, the attempt record would be marked as failed.

**Environment:** `AR_BASE_URL` (default: `http://localhost:8086`)

### AR (Event Consumer — handler exists, NATS loop not wired)

The `consumer.rs` module has a handler for `ar.invoice_suspended` events. When AR's dunning flow reaches terminal escalation for an invoice, the handler suspends the corresponding subscription(s). This is a one-way consumption — Subscriptions never calls AR to query dunning status. **Note:** The handler function exists but `main.rs` does not spawn a NATS subscription loop to invoke it — the NATS consumer wiring is not yet implemented.

### Payments (None — Explicit Boundary)

Subscriptions **never calls Payments directly**. Payment status flows through AR's dunning events. This is an intentional architectural boundary — Subscriptions owns billing schedules, AR owns invoices and dunning, Payments owns payment processing.

### GL (Event-Driven, One-Way — Future)

GL could subscribe to `subscriptions.status.changed` and `subscriptions.billrun.completed` for revenue recognition or billing analytics. Not implemented in v1.

### Notifications (Event-Driven, One-Way — Future)

Notifications could subscribe to:
- `subscriptions.status.changed` → send suspension/past-due alerts
- `subscriptions.billrun.completed` → send billing cycle summaries

Not implemented in v1.

---

## Invariants

1. **No invoice data stored.** Subscriptions never persists invoice IDs, amounts, or statuses in its domain tables. The `subscription_invoice_attempts` ledger records only the attempt status and AR invoice ID reference for cycle gating.
2. **No payment references stored.** Payment methods, transaction IDs, and payment statuses are never stored.
3. **Exactly-once invoice per cycle (module built, not wired).** The UNIQUE constraint on `(tenant_id, subscription_id, cycle_key)` in `subscription_invoice_attempts` prevents duplicate invoice attempts at the database level. Advisory locks prevent concurrent races. **However, the `execute_bill_run` endpoint does not yet use the gating module — it calls AR directly.**
4. **Lifecycle transitions are guard-protected (for active/past_due/suspended only).** Changes through `transition_guard()` validate the from→to pair. **Note:** `paused` and `cancelled` statuses exist in the DB CHECK constraint but are NOT covered by the guard — they have no transition functions.
5. **Outbox atomicity (partial).** `transition_to_past_due` and `transition_to_suspended` write events to the outbox in the same transaction as the status update. `transition_to_active` does **NOT** write to the outbox (no event emitted on reactivation). The `execute_bill_run` endpoint writes the `billrun.completed` event outside the bill run record update transaction.
6. **Event consumption is idempotent.** The `processed_events` table deduplicates incoming events. Processing the same event twice produces the same result.
7. **Bill run idempotency.** The `bill_run_id` UNIQUE constraint on `bill_runs` prevents the same bill run from executing twice.
8. **Subscriptions never calls Payments.** This boundary is enforced by design — no Payments client, no Payments URL configuration, no Payments-related code.
9. **Tenant isolation (partial).** `subscription_plans`, `subscriptions`, `subscription_invoice_attempts`, `events_outbox`, and `processed_events` have `tenant_id`. **`bill_runs` does NOT have a `tenant_id` column.** The `execute_bill_run` query for due subscriptions does NOT filter by `tenant_id` — it finds all active subscriptions across all tenants.

---

## API Surface (Summary)

Full OpenAPI contract: `contracts/subscriptions/subscriptions-v1.yaml`

**Note:** The OpenAPI contract defines the full planned API. Only routes marked with checkmarks below have implemented handlers.

### Subscription Plans (contract-only — no route handlers)
- `POST /api/subscription-plans` — Create subscription plan
- `GET /api/subscription-plans` — List subscription plans
- `GET /api/subscription-plans/{id}` — Get subscription plan

### Subscriptions (contract-only — no route handlers)
- `POST /api/subscriptions` — Create subscription
- `GET /api/subscriptions` — List subscriptions (filterable by customer_id, status)
- `GET /api/subscriptions/{id}` — Get subscription detail
- `POST /api/subscriptions/{id}/pause` — Pause subscription
- `POST /api/subscriptions/{id}/resume` — Resume subscription
- `POST /api/subscriptions/{id}/cancel` — Cancel subscription

### Bill Runs (implemented)
- `POST /api/bill-runs/execute` — Execute billing cycle (idempotent via bill_run_id) ✅

### Admin (implemented)
- `POST /api/subscriptions/admin/projection-status` — Query projection status ✅
- `POST /api/subscriptions/admin/consistency-check` — Run consistency check ✅
- `GET /api/subscriptions/admin/projections` — List projections ✅

### Operational (implemented)
- `GET /api/health` — Liveness check ✅
- `GET /api/ready` — Readiness probe (verifies DB connectivity) ✅
- `GET /api/version` — Module identity and schema version ✅
- `GET /metrics` — Prometheus metrics ✅
- `GET /healthz` — Kubernetes liveness ✅

### OpenAPI Contract Gaps
The contract's `Subscription.status` enum lists `[active, paused, cancelled]` but the database also supports `past_due` and `suspended`. The contract should be updated to include all 5 statuses.

---

## Decision Log

Every significant product, architecture, or standards decision is recorded here. Do not re-open a decision without adding a new row that supersedes the old one.

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-12 | Subscriptions never stores invoice data — delegates to AR via HTTP API | Prevents data divergence; AR is single source of truth for invoices; Subscriptions only needs to know "did billing succeed?" | Platform Orchestrator |
| 2026-02-12 | Bill run idempotency via bill_run_id UNIQUE constraint | Simple database-level guarantee; same bill_run_id returns cached result instead of re-processing | Platform Orchestrator |
| 2026-02-12 | Subscription status CHECK constraint includes paused and cancelled alongside active | Basic lifecycle needs for MVP; customer and admin can pause/cancel subscriptions | Platform Orchestrator |
| 2026-02-15 | Added past_due and suspended states for dunning lifecycle | Payment failure creates a grace period (past_due) before terminal suspension; AR's dunning events drive these transitions | Platform Orchestrator (Phase 15) |
| 2026-02-15 | Cycle gating: advisory locks + UNIQUE constraint for exactly-once invoice | Two-layer protection: advisory locks prevent races, UNIQUE constraint provides database-level guarantee; lock released before AR API calls to minimize contention | Platform Orchestrator (Phase 15) |
| 2026-02-15 | Guard → Mutation → Side Effect pattern for lifecycle transitions | Pure guard functions are testable without a database; side effects (events) only happen after guard approval; prevents orphaned state | Platform Orchestrator (Phase 15) |
| 2026-02-15 | Idempotent same-state transitions (active→active, etc.) | Prevents errors on event replay; processing the same dunning event twice should not fail | Platform Orchestrator (Phase 15) |
| 2026-02-15 | Attempt ledger records kept even on failure (status: failed_final) | Full audit trail; enables monitoring of AR API reliability; supports recovery of stuck 'attempting' records | Platform Orchestrator (Phase 15) |
| 2026-02-15 | Active→Suspended direct transition allowed (dunning terminal escalation) | Some tenants may want immediate suspension without a past_due grace period; keeps the state machine flexible | Platform Orchestrator (Phase 15) |
| 2026-02-15 | Suspended→Past_Due transition is illegal | Cannot go backwards in escalation; recovery from suspended goes directly to active | Platform Orchestrator (Phase 15) |
| 2026-02-16 | Outbox enriched with full envelope metadata columns | Makes events queryable by tenant_id, trace_id, mutation_class without deserializing payload; supports distributed tracing and replay analysis | Platform Orchestrator (Phase 16) |
| 2026-02-16 | Lifecycle transitions wrapped in transactions for atomicity (bd-299f) | Status update + outbox event insert must commit atomically; prevents orphaned status changes without corresponding events | Platform Orchestrator (Phase 16) |

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`
