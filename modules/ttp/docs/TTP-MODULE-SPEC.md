# TTP Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Proven Module (v1.0.0)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | CopperRiver | Initial vision doc — documented from source code, migrations, tests, OpenAPI contract, and REVISIONS.md. Business problem, domain authority, data ownership, events, integration points, invariants, API surface, structural decisions, decision log. |

---

## The Business Problem

Multi-tenant SaaS platforms that serve downstream parties (customers, franchises, locations) face a universal billing problem: **how do you turn a tenant's service agreements and metered usage into correct, auditable invoices?**

A property management company has 200 units on different plans. A franchise network has 35 locations each paying a base fee plus metered API calls. A waste hauler bills customers monthly based on container pickups plus per-ton disposal charges. In every case, the platform operator needs to:

1. Track which parties have which plans and at what price.
2. Record granular usage events (API calls, storage, pickups, transactions) as they happen.
3. At billing time, aggregate usage, apply pricing rules, combine with recurring fees and one-time charges, and produce one invoice per party.
4. Do all of this idempotently — a billing run that crashes mid-way and restarts must not double-bill anyone.

Without a dedicated billing pipeline, platform operators build fragile scripts, lose money to billing errors, and spend engineering cycles debugging invoices instead of building product. TTP solves this by providing a deterministic, idempotent billing engine that bridges tenant provisioning and the AR (Accounts Receivable) module.

---

## What the Module Does

TTP (Tenant-to-Party) is the **authoritative billing pipeline** between platform-level tenant identity and the AR module's invoice machinery. It answers three questions:

1. **What does each party owe?** — Service agreements define recurring charges per party. One-time charges capture ad-hoc fees. Metering events track granular usage.
2. **How much does the usage cost?** — A deterministic price trace engine aggregates metering events by dimension, applies tenant-scoped pricing rules, and computes line totals using integer arithmetic (no rounding, no floating point).
3. **Has this period been billed?** — Billing runs are idempotent per (tenant, period). Re-running a completed period is a no-op. Crashed runs resume from where they left off. One-time charges are marked billed only after the AR invoice is finalized.

---

## Who Uses This

TTP is an internal platform service. It has no frontend of its own — it exposes an API consumed by other platform services and orchestration workflows.

### Platform Billing Orchestrator
- Triggers billing runs per tenant per period
- Receives billing run completion/failure events
- Monitors trace-to-invoice equivalence

### Tenant Administrators
- Manage service agreements (plans) for their parties
- Review metering traces to verify usage-based charges before invoicing
- Submit one-time charges for ad-hoc billing

### System (Metering Ingestion)
- Other platform services emit metering events (API call counts, storage usage, transaction volumes)
- Events are ingested via the metering endpoint with per-event idempotency keys
- Events accumulate until a billing run aggregates them

### AR Module (Downstream Consumer)
- Receives invoice creation and finalization commands from TTP during billing runs
- AR is unaware of TTP's domain — it simply processes invoice requests

---

## Design Principles

### Deterministic Billing — Same Inputs, Same Output, Always
The price trace computation is fully deterministic. Events are aggregated by dimension with stable ordering (dimension name, then event_id). Pricing rules are resolved by effective date with latest-wins semantics. Line totals use integer multiplication (`quantity * unit_price_minor`) — no floating-point arithmetic, no rounding. Running the same trace twice produces byte-identical output, and the SHA-256 hash of the serialized trace is stored on the billing run item for audit linkage.

### Idempotent at Every Layer
- **Metering ingestion:** `ON CONFLICT (tenant_id, idempotency_key) DO NOTHING` — duplicate events are silently absorbed.
- **Billing runs:** `UNIQUE (tenant_id, billing_period)` — one run per period. Re-calling a completed run returns the existing summary with `was_noop: true`.
- **Billing run items:** `UNIQUE (run_id, party_id)` — one item per party per run. Upsert on conflict for crash recovery.
- **AR invoice creation:** Correlation ID (SHA-256 of `run_id/party_id`) serves as the idempotency key for the AR create-invoice call.

### Fail-Closed on External Dependencies
TTP calls two external services: tenant-registry (to resolve `tenant_id` → `app_id`) and AR (to create invoices). Both are fail-closed — if either is unreachable or returns an error, the billing run aborts rather than producing partial results. There is no degraded mode for billing. Money operations do not tolerate silent failures.

### One-Time Charges Are Marked Billed Post-Invoice
One-time charges transition from `pending` → `billed` only after the AR invoice has been finalized. If the process crashes between invoice creation and charge marking, the next run re-entry detects the already-invoiced item and skips it, then marks the charges. This prevents double-billing even in crash scenarios.

---

## MVP Scope (v1.0.0 — Current)

### In Scope
- TTP customer register (lightweight party reference with tenant-scoped status)
- Service agreements: recurring billing plans per party (monthly/quarterly/annual cycles)
- One-time charges: ad-hoc charges included in the next billing run
- Metering event ingestion (batch, idempotent, per-event deduplication)
- Metering pricing rules with effective date ranges
- Deterministic price trace computation (aggregate by dimension, apply pricing, integer arithmetic)
- Billing run execution: merge agreement amounts + one-time charges + metered usage → AR invoices
- Billing run idempotency: one run per (tenant, period), crash-safe re-entry
- Trace-to-invoice linkage via SHA-256 trace_hash on billing run items
- Tenant-registry integration (resolve tenant_id → app_id, fail-closed)
- AR integration (find-or-create customer, create draft invoice, finalize invoice)
- 4 domain events emitted via EventEnvelope (see Events Produced)
- OpenAPI contract: `contracts/ttp/ttp-v1.0.0.yaml`
- Health (`/healthz`, `/api/health`), readiness (`/api/ready`), version (`/api/version`), metrics (`/metrics`)
- Prometheus SLO metrics (request latency histogram, request counter, event consumer lag)
- Docker production image (multi-stage, cargo-chef cached)

### Explicitly Out of Scope for v1
- Proration (mid-cycle plan changes)
- Tiered or graduated pricing (volume discounts, usage tiers)
- Multi-currency billing within a single run
- Credit notes and refunds
- Dunning / payment retry orchestration
- Usage alerts and budget thresholds
- Self-service plan management API (CRUD for agreements)
- Scheduled/automatic billing run triggers (cron or scheduler)
- NATS event bus publishing (envelopes are created but not yet wired to bus)
- Frontend UI

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum | Port 8100 (default) |
| Database | PostgreSQL | Dedicated database (`ttp_{app_id}_db` naming convention), SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate; envelope creation implemented, bus publishing deferred |
| Auth | JWT via platform `security` crate | AuthzLayer, rate limiting, timeout middleware |
| HTTP clients | reqwest | AR module and tenant-registry communication |
| Metrics | Prometheus | Request latency, request count, event consumer lag |
| Hashing | SHA-256 (sha2 crate) | Billing item idempotency keys, trace hash computation |
| Crate | `ttp-rs` | Single crate, binary name `ttp` |

---

## Structural Decisions (The "Walls")

### 1. TTP is a billing pipeline, not a billing engine
TTP does not own invoice rendering, payment processing, or ledger posting. It computes what is owed, then delegates invoice creation to AR. This separation means AR can evolve its invoice model (line items, taxes, discounts) without changing TTP, and TTP can evolve its pricing logic without touching AR.

### 2. Integer arithmetic for all monetary computation
All amounts are stored and computed in minor currency units (cents for USD). `line_total = quantity * unit_price_minor` — integer multiplication, no floating-point, no rounding. This eliminates an entire class of billing discrepancy bugs. Currency is stored as a 3-character ISO 4217 code alongside every monetary field.

### 3. Metering and billing are separate domains within TTP
Metering (event ingestion, aggregation, pricing rules, trace computation) and billing (run execution, AR integration, charge marking) are separate domain modules with separate DB tables and separate HTTP handlers. They share a crate but have independent error types. The billing module calls metering's `compute_price_trace` as a read-only operation — metering never knows about billing.

### 4. Idempotency keys are derived, not stored
The billing run item's idempotency key for AR invoice creation is `SHA-256(run_id/party_id)` — derived deterministically, not stored as a separate column. This means the key is always reproducible from the run and party, eliminating the possibility of key mismatch or orphaned keys.

### 5. One billing run per (tenant, period) — no partial reruns
The `UNIQUE (tenant_id, billing_period)` constraint on `ttp_billing_runs` ensures exactly one run per period. There is no mechanism to "re-bill a single party" — the entire run is atomic. This trades flexibility for correctness: you cannot accidentally bill party A twice while skipping party B.

### 6. Tenant-registry lookup is fail-closed
Before creating a billing run, TTP resolves `tenant_id` → `app_id` via the tenant-registry. If the registry is unreachable or the tenant has no `app_id`, the run aborts with a clear error. There is no fallback or cache. This prevents billing against misconfigured or unknown tenants.

### 7. Trace hash provides audit linkage
When a billing run item originates from metered usage, the SHA-256 hash of the serialized PriceTrace is stored in `trace_hash`. This allows any auditor to recompute the trace independently and verify it matches the stored hash. Non-metering items (agreement-only) have `NULL` trace_hash.

### 8. Tenant isolation via tenant_id on every table
Standard platform multi-tenant pattern. Every table has `tenant_id` as a non-nullable field. Every query filters by `tenant_id`. No cross-tenant data leakage.

### 9. No mocking in tests
Integration tests hit real Postgres, real AR, real tenant-registry. Tests that mock the database or HTTP clients test nothing useful. This is a platform-wide standard.

---

## Domain Authority

TTP is the **source of truth** for:

| Domain Entity | TTP Authority |
|---------------|--------------|
| **TTP Customers** | Lightweight per-tenant party register with status (active/suspended/cancelled). Links to Party module via `party_id`. |
| **Service Agreements** | Recurring billing plans per party: plan code, amount, currency, billing cycle, effective dates, status. |
| **One-Time Charges** | Ad-hoc charges per party with pending/billed/cancelled lifecycle. Marked billed only after AR invoice finalization. |
| **Metering Events** | Raw usage data points: tenant, dimension, quantity, timestamp, idempotency key, optional source reference. |
| **Metering Pricing** | Per-dimension pricing rules with effective date ranges. Used for price trace computation. |
| **Billing Runs** | One run per (tenant, period). Tracks status (pending → processing → completed/failed) and idempotency key. |
| **Billing Run Items** | One item per party per run. Links to AR invoice via UUID, carries amount and trace_hash. |
| **Price Traces** | Computed (not stored) aggregations of metering events × pricing rules. Deterministic output, hashable for audit. |

TTP is **NOT** authoritative for:
- Invoice rendering, payment status, or AR ledger balances (AR module owns this)
- Tenant identity, app_id assignment, or provisioning state (tenant-registry owns this)
- Party master data — name, address, contact info (Party module owns this)
- GL journal entries or expense tracking (GL module owns this)
- Payment processing or gateway interactions (Payments module owns this)

---

## Data Ownership

### Tables Owned by TTP

All tables use `tenant_id` for multi-tenant isolation. Every query **MUST** filter by `tenant_id`.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **ttp_customers** | Lightweight party register | `id`, `tenant_id`, `party_id` (UNIQUE per tenant), `external_ref`, `status` (active\|suspended\|cancelled) |
| **ttp_service_agreements** | Recurring billing plans | `agreement_id`, `tenant_id`, `party_id`, `plan_code`, `amount_minor`, `currency` (CHAR 3), `billing_cycle` (monthly\|quarterly\|annual), `status` (active\|suspended\|cancelled), `effective_from`, `effective_to` |
| **ttp_one_time_charges** | Ad-hoc charges | `charge_id`, `tenant_id`, `party_id`, `description`, `amount_minor`, `currency`, `status` (pending\|billed\|cancelled), `ar_invoice_id` (set when billed) |
| **ttp_billing_runs** | One run per (tenant, period) | `run_id`, `tenant_id`, `billing_period` (UNIQUE per tenant), `status` (pending\|processing\|completed\|failed), `idempotency_key` |
| **ttp_billing_run_items** | One item per party per run | `id`, `run_id` (FK → billing_runs), `party_id` (UNIQUE per run), `ar_invoice_id`, `amount_minor`, `currency`, `status` (pending\|invoiced\|failed), `trace_hash` (nullable) |
| **ttp_metering_events** | Raw usage data | `event_id`, `tenant_id`, `dimension`, `quantity` (BIGINT > 0), `occurred_at`, `idempotency_key` (UNIQUE per tenant), `source_ref`, `ingested_at` |
| **ttp_metering_pricing** | Per-dimension pricing rules | `pricing_id`, `tenant_id`, `dimension`, `unit_price_minor`, `currency`, `effective_from` (UNIQUE per tenant+dimension), `effective_to` |
| **ttp_processed_events** | Event deduplication (NATS consumption) | `id`, `event_id` (UNIQUE), `event_type`, `processed_at` |

**Monetary Precision:** All monetary amounts use **integer minor units** (e.g., `amount_minor` in cents). Currency stored as 3-letter ISO 4217 code.

**Tenant Isolation:** Every table includes `tenant_id` as a non-nullable field. Unique constraints incorporate `tenant_id` as a leading column.

### Data NOT Owned by TTP

TTP **MUST NOT** store:
- AR invoice details beyond the opaque `ar_invoice_id` UUID reference
- Payment status, gateway responses, or settlement data
- Party master data (name, address, contact info)
- Tenant provisioning state, app_id, or product codes (queried at runtime, not cached)
- GL account codes or journal entry details

---

## Events Produced

All events use the platform `EventEnvelope` with `merchant_context = TENANT(tenant_id)` to enforce money-mixing prevention. Events are created via the `create_ttp_envelope` helper.

| Event | NATS Subject | Trigger | Key Payload Fields |
|-------|-------------|---------|-------------------|
| Billing Run Created | `ttp.billing_run.created` | Billing run record inserted | `run_id`, `tenant_id`, `billing_period`, `idempotency_key` |
| Billing Run Completed | `ttp.billing_run.completed` | Billing run finishes successfully | `run_id`, `tenant_id`, `billing_period`, `parties_billed`, `total_amount_minor`, `currency` |
| Billing Run Failed | `ttp.billing_run.failed` | Billing run encounters an error | `run_id`, `tenant_id`, `billing_period`, `reason` |
| Party Invoiced | `ttp.party.invoiced` | Individual party billed within a run | `run_id`, `tenant_id`, `party_id`, `ar_invoice_id`, `amount_minor`, `currency` |

**Note:** In v1.0.0, event envelopes are created and merchant_context is validated, but NATS bus publishing is not yet wired. A future bead will complete end-to-end event delivery.

---

## Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| *(None in v1)* | — | TTP is event-producing only in v1. Future: consume `tenant.provisioned` to auto-create TTP customer records. |

---

## Integration Points

### Tenant-Registry (Required, HTTP Query)

Before executing a billing run, TTP resolves `tenant_id` → `app_id` via `GET /api/tenants/{tenant_id}/app-id`. This is fail-closed: if the registry is unreachable or returns 404 (unknown tenant) or 409 (no app_id), the billing run aborts. The `app_id` is used to scope AR operations. TTP never caches this mapping.

### AR Module (Required, HTTP Commands)

TTP creates invoices in AR during billing runs:
1. `GET /api/ar/customers?external_customer_id={party_id}` — find existing AR customer
2. `POST /api/ar/customers` — create AR customer if not found (external_customer_id = party_id)
3. `POST /api/ar/invoices` — create draft invoice with idempotency key and amount
4. `POST /api/ar/invoices/{id}/finalize` — move draft → open

AR uses integer IDs internally. TTP stores a UUID5 derived from the AR invoice ID as `ar_invoice_id` on billing run items. **Degradation:** If AR is unreachable, the billing run fails entirely — no partial invoicing.

### Party Module (Informational)

`ttp_customers.party_id` references a party UUID. TTP does not call the Party module at runtime — the reference is set at customer registration time. Party master data (name, address) is not duplicated in TTP.

### Platform Event Bus (One-Way, Deferred)

Event envelopes are created with the correct `MerchantContext::Tenant` tagging, but NATS publishing is not yet connected in v1.0.0. Downstream consumers (notifications, analytics) will subscribe to TTP events once bus wiring is complete.

---

## Invariants

1. **Tenant isolation is unbreakable.** Every query filters by `tenant_id`. No cross-tenant data leakage.
2. **One billing run per (tenant, period).** `UNIQUE (tenant_id, billing_period)` on `ttp_billing_runs` — enforced at the database level.
3. **One billing item per (run, party).** `UNIQUE (run_id, party_id)` on `ttp_billing_run_items` — prevents double-invoicing within a run.
4. **Metering ingestion is exactly-once per idempotency key.** `UNIQUE (tenant_id, idempotency_key)` with `ON CONFLICT DO NOTHING` — duplicate events are silently absorbed.
5. **Price trace is deterministic.** Same metering events + same pricing rules = same trace output. Integer arithmetic only. Sorted by dimension for stable serialization.
6. **One-time charges are marked billed only after AR invoice finalization.** The UPDATE happens after `finalize_invoice` returns success. Crash between create and finalize → next run re-entry detects the existing item.
7. **Billing fails closed on external dependency errors.** Tenant-registry unreachable → run aborts. AR unreachable → run aborts. No partial results.
8. **Monetary values are always integer minor units.** No floating-point anywhere in the billing path. Currency is 3-char ISO 4217.
9. **Merchant context is always TENANT-scoped on events.** All event envelopes carry `merchant_context = TENANT(tenant_id)` — enforced by the `create_ttp_envelope` helper.

---

## API Surface (Summary)

Full OpenAPI contract: `contracts/ttp/ttp-v1.0.0.yaml`

### Billing Runs
- `POST /api/ttp/billing-runs` — Trigger a billing run for a tenant + period. Idempotent: re-calling a completed period returns `was_noop: true`.

### Metering
- `POST /api/metering/events` — Ingest batch of metering events (per-event idempotency). Validates all events before writing any.
- `GET /api/metering/trace` — Compute deterministic price trace for a tenant + period. Query params: `tenant_id`, `period` (YYYY-MM).

### Service Agreements
- `GET /api/ttp/service-agreements` — List service agreements for a tenant. Filterable by `status` (active/suspended/cancelled/all). Sorted by `plan_code` then `agreement_id`.

### Operational
- `GET /healthz` — Liveness probe (no dependency checks)
- `GET /api/health` — Health check with service identity and version
- `GET /api/ready` — Readiness probe (verifies DB connectivity with latency measurement)
- `GET /api/version` — Module name, version, schema version
- `GET /metrics` — Prometheus text format metrics

---

## Decision Log

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-19 | TTP owns its own customer register separate from Party module | TTP needs tenant-scoped party status (active/suspended/cancelled) for billing gating; Party module's status semantics may differ | Platform Orchestrator |
| 2026-02-19 | Service agreement status and one-time charge lifecycle are separate from billing run status | A cancelled agreement does not cancel an in-progress billing run; charge status tracks billing completion, not business validity | Platform Orchestrator |
| 2026-02-19 | Billing run idempotency at (tenant_id, billing_period) granularity | Period-level idempotency is the simplest model that prevents double-billing; per-party reruns add complexity with no clear benefit in v1 | Platform Orchestrator |
| 2026-02-19 | UNIQUE constraint on billing_run_items (run_id, party_id) added as separate migration | Originally missing from initial schema; discovered during billing implementation that ON CONFLICT upsert required it for crash recovery | Platform Orchestrator |
| 2026-02-20 | Metering events use (tenant_id, idempotency_key) as deduplication key | Per-tenant scoping allows different tenants to use the same key space; idempotency_key is caller-supplied for exactly-once guarantee | Platform Orchestrator |
| 2026-02-20 | Metering pricing uses effective date ranges, not simple key-value | Price changes must take effect at period boundaries without losing historical pricing; DISTINCT ON (dimension) ORDER BY effective_from DESC resolves the correct rule | Platform Orchestrator |
| 2026-02-20 | Trace hash stored on billing_run_items for audit linkage | Allows independent trace recomputation and verification; non-metering items have NULL trace_hash to distinguish agreement-only billing | Platform Orchestrator |
| 2026-02-20 | Metering aggregation uses half-open interval [period_start, period_end) | Standard convention for time range queries; prevents boundary ambiguity (midnight events belong to the new period) | Platform Orchestrator |
| 2026-02-22 | Module proven at v1.0.0 with E2E tests covering metering + billing integration | proof_ttp.sh script validates idempotent metering ingestion, deterministic trace computation, and trace-to-invoice equivalence with real Postgres and real AR service | Platform Orchestrator |
| 2026-02-22 | AR invoice idempotency key is SHA-256(run_id/party_id), not stored separately | Derived keys are always reproducible; eliminates key storage, mismatch, and orphan risks | Platform Orchestrator |
| 2026-02-22 | Event bus publishing deferred — envelopes created but not wired to NATS | Core billing pipeline proven without event delivery; wiring can be added without changing the billing domain logic | Platform Orchestrator |

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`
