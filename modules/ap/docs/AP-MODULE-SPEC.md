# AP (Accounts Payable) Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.0)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | CopperRiver | Initial vision doc — documented from existing source code, migrations, events, and tests. Covers full procure-to-pay lifecycle, 3-way match engine, payment runs, tax integration, and aging reports. |
| 1.1 | 2026-02-24 | PurpleCliff | Fresh-eyes review: fixed tenant_id claims (child tables inherit via FK, not direct column), added missing columns to table descriptions (entered_at, matched_by/matched_at, tax lifecycle timestamps, executed_at, quantity_received type), added Admin routes to API Surface. |

---

## The Business Problem

Every organization that buys goods or services faces the same challenge: **paying vendors accurately, on time, and with full audit visibility.** The procure-to-pay cycle — from issuing a purchase order through receiving goods to approving and paying the vendor's invoice — is the single largest source of cash outflow for most businesses.

When this process is manual, problems compound. Duplicate invoices get paid because nobody cross-references the PO. Goods arrive but the receipt is never matched to the bill, so discrepancies go undetected. Payment terms slip because nobody tracks due dates systematically, resulting in late fees or missed early-payment discounts. AP aging reports are stale spreadsheets, and cash flow forecasting operates on guesses.

Small and mid-size businesses either lack AP automation entirely — paying from email inboxes and paper — or use legacy ERP systems that bundle AP into a monolith with rigid workflows and six-figure license fees. The businesses that need cost control most have the least tooling for it.

---

## What the Module Does

The AP module is the **authoritative system for the vendor-side of financial obligations**: purchase orders, vendor bills, 3-way matching, payment allocation, and disbursement orchestration. It implements the full procure-to-pay lifecycle for multi-tenant SaaS.

It answers six questions:
1. **Who do we buy from?** — A vendor register with payment terms, currency, payment method, and optional party-master linkage.
2. **What did we order?** — Purchase orders with line-level detail, GL account routing, approval gating, and status tracking.
3. **Did we receive what we ordered?** — Receipt linkage from the Inventory module, anchoring the 3-way match (PO / receipt / bill).
4. **Does the bill match the order?** — A deterministic match engine comparing bill lines to PO lines and received quantities, with configurable price tolerance.
5. **What do we owe and when?** — Approved bills with deterministic due-date derivation, aging buckets (current / 1-30 / 31-60 / 61-90 / 90+), and open balance tracking.
6. **How do we pay?** — Payment runs that batch eligible bills by vendor and currency, execute disbursements, and record allocations.

---

## Who Uses This

The module is a platform service consumed by any vertical application managing vendor payables. It does not have its own frontend — it exposes an API.

### AP Manager / Controller
- Registers vendors with payment terms, currency, and payment method
- Creates and approves purchase orders
- Enters vendor bills with line-level GL account routing
- Runs the match engine to verify bills against POs and receipts
- Approves bills for payment (with match policy override when needed)
- Reviews AP aging reports by currency and vendor
- Creates and executes payment runs

### Procurement / Purchasing
- Creates purchase orders against registered vendors
- Tracks PO status (draft / approved / closed / cancelled)
- Reviews receipt linkages against PO lines

### Operations / Warehouse
- Receipt events from Inventory auto-link to PO lines (single-line POs)
- Multi-line PO receipt linkage via explicit API

### Finance / GL Consumer
- Receives `ap.vendor_bill_approved` events with per-line GL account allocations
- Posts journal entries: DR Expense / CR AP Liability
- Receives `ap.vendor_bill_voided` for reversals
- Receives `ap.payment_executed` for payment clearing entries

### System (Event Consumers)
- Inventory module publishes `inventory.item_received` — AP consumer ingests receipt links
- GL module subscribes to AP events for journal posting
- Notifications module subscribes to AP events for alerts

---

## Design Principles

### Full Procure-to-Pay in One Module
The AP module owns the complete lifecycle: vendor register, purchase orders, receipt linkage, bill entry, 3-way matching, approval, payment allocation, and disbursement coordination. No partial implementations — every step in the chain is connected through a consistent data model and event contract.

### Append-Only Financial Records
AP allocations are append-only — no UPDATE, no DELETE. Once a payment is applied to a bill, it is an immutable audit record. Bill voiding is a compensating event (REVERSAL mutation class), not a delete. This ensures the financial ledger can always be reconstructed from events.

### Deterministic Due-Date Derivation
Bill due dates are computed deterministically from the invoice date and vendor payment terms (`invoice_date + payment_terms_days`). This is a pure function — the same inputs always produce the same date. Explicit due dates override the computed default when provided.

### Match Policy Before Payment
Bills cannot proceed to payment without either passing the match engine (PO lines / receipts / bill lines compared within tolerance) or receiving an explicit approval override with documented reason. The match engine itself does not auto-approve — it computes and stores results for human decision-making.

### Event-Driven GL Integration
AP never calls GL. All financial events carry self-contained payloads (replay-safe) with per-line GL account codes, amounts, and FX metadata. GL subscribes to NATS subjects and posts journal entries autonomously. This means AP functions without GL running.

### Idempotency at Every Seam
Every write operation has an idempotency anchor: `allocation_id` for payments, `vendor_invoice_ref` per vendor per tenant for bills, `po_number` per tenant for POs, `(po_line_id, receipt_id)` for receipt links, `(run_id, item_id)` for payment executions, deterministic UUID v5 for payment IDs. Retries are always safe.

---

## Current Scope (v0.1.0)

### In Scope
- Vendor register (CRUD, soft deactivation, duplicate name detection, party-master linkage)
- Payment terms presets (Net-0 through Net-90) and custom days
- Purchase orders with line-level detail, GL account codes, and status machine (draft / approved / closed / cancelled)
- PO approval with audit trail (po_status append-only log)
- Receipt linkage from Inventory events (single-line PO auto-inference)
- Vendor bill entry with line-level detail, GL account routing, and FX rate reference
- Bill status machine (open / matched / approved / partially_paid / paid / voided)
- 3-way match engine: two_way (PO / bill), three_way (PO / receipt / bill), non_po (bill only)
- Match tolerance: configurable price tolerance (default 5%), exact quantity match
- Bill approval with match policy enforcement and override capability
- Bill voiding with compensating REVERSAL events
- Payment allocations (append-only, partial/full, open balance derivation)
- Payment runs: batch creation by currency, vendor selection, due-date filtering
- Payment run execution: per-vendor disbursement with deterministic payment IDs
- AP aging report: current / 1-30 / 31-60 / 61-90 / 90+ buckets by currency and vendor
- Tax integration via `tax-core` TaxProvider trait (quote / commit / void lifecycle)
- AP tax snapshots (per-bill, at most one active non-voided snapshot)
- HTTP idempotency keys with TTL-based expiry
- 12 domain events via outbox (see Events Produced)
- Prometheus metrics: SLO histograms, business gauges, consumer lag
- NATS consumer for `inventory.item_received`
- Admin routes

### Explicitly Out of Scope
- Credit memos / debit memos
- Vendor self-service portal
- Automated approval workflows (rules engine)
- Multi-currency payment runs (currently single-currency per run)
- Recurring / scheduled bill entry
- Document / receipt image attachment
- Vendor performance scoring
- Budget checking at PO creation
- Petty cash / expense reimbursement

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum | Port 8095 (default) |
| Database | PostgreSQL | Dedicated database, SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate; InMemory fallback for dev |
| Auth | JWT via platform `security` crate | Tenant-scoped, role-based; `AP_MUTATE` permission for writes |
| Outbox | Platform outbox pattern | Same as all other modules |
| Metrics | Prometheus | `/metrics` endpoint, SLO histograms + business gauges |
| Tax | `tax-core` crate | TaxProvider trait; ZeroTaxProvider default |
| Projections | `projections` crate | Platform projection infrastructure |
| Crate | `ap` | Single crate, modular domain layout |

---

## Structural Decisions (The "Walls")

### 1. Vendor register is AP-owned, party-master is optional
AP owns the `vendors` table with all payment-relevant fields (terms, currency, payment method, remittance). An optional `party_id` links to an external party-master service for cross-module identity (e.g., a vendor who is also a customer), but AP never calls the party-master at runtime. The reference is informational.

### 2. Purchase orders are commitments, not accounting events
POs represent purchasing intent — they do not create financial liabilities. The AP liability is only created when a vendor bill is entered. PO status transitions (draft / approved / closed / cancelled) are lifecycle events, not financial mutations. This separation keeps the GL clean.

### 3. 3-way match is deterministic and non-blocking
The match engine computes results and stores them — it does not block bill processing. A bill can be approved without matching (with an override reason). This design avoids the common ERP problem where match failures create bottlenecks in the AP pipeline.

### 4. Allocations are append-only
The `ap_allocations` table has a strict NO UPDATE, NO DELETE policy. Payment application is modeled as immutable allocation records. Bill status (`partially_paid` / `paid`) is derived deterministically from the sum of allocations against the bill total. This makes the payment history fully auditable and replayable.

### 5. Payment ID is deterministic (UUID v5 from run_id + vendor_id)
The Payments integration seam derives `payment_id` using UUID v5 namespaced on `run_id:vendor_id`. This means re-executing a payment run for the same vendor always produces the same `payment_id`, making the integration naturally idempotent. No round-trip to the Payments module is needed for idempotency.

### 6. Bill voiding is a compensating event, not a delete
Voiding a bill emits a REVERSAL-class event with `reverses_event_id` pointing to the original creation event. The GL consumer uses this to post reversing journal entries. The bill record remains in the database with `status = 'voided'` — nothing is deleted.

### 7. All events are replay-safe and self-contained
Every AP event payload contains all data needed for downstream consumers to process it without querying the AP database. The `ap.vendor_bill_approved` event carries per-line GL account codes and amounts, FX rate references, and vendor details. This is the platform EventEnvelope standard.

### 8. Tenant isolation via tenant_id on primary tables
Standard platform multi-tenant pattern. Primary tables have `tenant_id` as a non-nullable field. Child tables (`po_lines`, `po_status`, `bill_lines`, `three_way_match`, `po_receipt_links`, `payment_run_items`, `payment_run_executions`) inherit tenant scope through FK relationships. Every query filters by `tenant_id`. Unique constraints include `tenant_id` where relevant.

### 9. No mocking in tests
Integration tests hit real Postgres. Tests that mock the database test nothing useful. This is a platform-wide standard.

---

## Domain Authority

AP is the **source of truth** for:

| Domain Entity | AP Authority |
|---------------|-------------|
| **Vendors** | Payment-relevant identity: name, tax ID, currency, payment terms, payment method, remittance email, active status. Optional party-master linkage. |
| **Purchase Orders** | Commitments to vendors: PO number, lines, quantities, unit prices, GL account codes, expected delivery, status lifecycle. |
| **PO Lines** | Line-level detail on purchase orders: description, quantity, unit of measure, unit price, GL account code. |
| **PO Status History** | Append-only audit trail of PO status transitions with actor attribution. |
| **Receipt Links** | AP-side linkage between PO lines and external receipt/GRN identifiers from the Inventory module. |
| **Vendor Bills** | AP liability records: vendor invoice reference, currency, total, tax, invoice date, due date, status lifecycle, FX rate reference. |
| **Bill Lines** | Line-level detail on vendor bills: description, quantity, unit price, GL account code, PO line reference. |
| **3-Way Match Records** | Match results linking bill lines to PO lines and receipts: match type, matched quantities, price/qty variances, tolerance results. |
| **Payment Allocations** | Append-only records of how payments are applied to bills: amount, currency, allocation type (partial/full). |
| **Payment Runs** | Batch payment proposals: scheduled date, payment method, per-vendor items, execution status. |
| **Payment Run Items** | Per-vendor detail within a payment run: bills selected, amount to pay. |
| **Payment Run Executions** | Per-item disbursement outcomes: payment ID, amount, status (success/failed), bank reference. |
| **AP Tax Snapshots** | Tax lifecycle per bill: quoted / committed / voided, provider references, per-line tax breakdowns. |
| **Aging Buckets** | Computed read model: open balance per bill (total - allocations), bucketed by days past due. |

AP is **NOT** authoritative for:
- Inventory stock levels, receipts, or warehouse operations (Inventory module owns this)
- GL account balances or journal entries (GL module owns this)
- FX rates or currency conversion rules (GL FX infrastructure owns this)
- Customer-facing invoices or receivables (AR module owns this)
- Disbursement processing, bank connectivity, or payment settlement (Payments module owns this)
- Party-master identity or cross-module entity resolution (Party-master service owns this)

---

## Data Ownership

### Tables Owned by AP

Primary tables use `tenant_id` for multi-tenant isolation. Every query **MUST** filter by `tenant_id`. Child tables (`po_lines`, `po_status`, `bill_lines`, `three_way_match`, `po_receipt_links`, `payment_run_items`, `payment_run_executions`) inherit tenant scope through FK relationships to their parent tables.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **vendors** | Vendor/supplier register | `vendor_id`, `tenant_id`, `name`, `tax_id`, `currency` (CHAR 3), `payment_terms_days`, `payment_method`, `remittance_email`, `is_active`, `party_id` (nullable UUID) |
| **purchase_orders** | PO headers | `po_id`, `tenant_id`, `vendor_id` (FK), `po_number` (unique per tenant), `currency`, `total_minor` (BIGINT), `status` (draft\|approved\|closed\|cancelled), `created_by`, `expected_delivery_date` |
| **po_lines** | PO line items | `line_id`, `po_id` (FK), `description`, `quantity` (NUMERIC 18,6), `unit_of_measure`, `unit_price_minor` (BIGINT), `line_total_minor`, `gl_account_code` |
| **po_status** | Append-only PO status audit log | `id` (BIGSERIAL), `po_id` (FK), `status`, `changed_by`, `changed_at`, `reason` |
| **po_receipt_links** | PO line to receipt/GRN linkage | `id` (BIGSERIAL), `po_id` (FK), `po_line_id` (FK), `vendor_id` (FK), `receipt_id`, `quantity_received` (NUMERIC 18,6), `unit_of_measure`, `unit_price_minor`, `currency`, `gl_account_code`, `received_at`, `received_by`; UNIQUE (`po_line_id`, `receipt_id`) |
| **vendor_bills** | AP liability records | `bill_id`, `tenant_id`, `vendor_id` (FK), `vendor_invoice_ref`, `currency`, `total_minor` (BIGINT), `tax_minor` (BIGINT, nullable), `invoice_date`, `due_date`, `status` (open\|matched\|approved\|partially_paid\|paid\|voided), `fx_rate_id` (nullable UUID), `entered_by`, `entered_at` |
| **bill_lines** | Bill line items | `line_id`, `bill_id` (FK), `description`, `quantity` (DOUBLE PRECISION), `unit_price_minor` (BIGINT), `line_total_minor`, `gl_account_code`, `po_line_id` (FK, nullable) |
| **three_way_match** | Match engine results | `id` (BIGSERIAL), `bill_id` (FK), `bill_line_id` (FK, UNIQUE), `po_id` (FK, nullable), `po_line_id` (FK, nullable), `receipt_id` (nullable), `match_type` (two_way\|three_way\|non_po), `matched_quantity`, `matched_amount_minor`, `within_tolerance`, `price_variance_minor`, `qty_variance`, `match_status` (matched\|price_variance\|qty_variance\|price_and_qty_variance), `matched_by`, `matched_at` |
| **ap_allocations** | Append-only payment application | `id` (BIGSERIAL), `allocation_id` (UUID, UNIQUE), `bill_id` (FK), `payment_run_id` (FK, nullable), `tenant_id`, `amount_minor` (BIGINT, > 0), `currency`, `allocation_type` (partial\|full) |
| **payment_runs** | Batch payment headers | `run_id`, `tenant_id`, `total_minor`, `currency`, `scheduled_date`, `payment_method`, `status` (pending\|executing\|completed\|failed), `created_by`, `executed_at` |
| **payment_run_items** | Per-vendor payment items | `id` (BIGSERIAL), `run_id` (FK), `vendor_id`, `bill_ids` (UUID[]), `amount_minor` (BIGINT), `currency` |
| **payment_run_executions** | Per-item execution outcomes | `id` (BIGSERIAL), `run_id` (FK), `item_id` (FK), `payment_id`, `vendor_id`, `amount_minor`, `currency`, `status` (success\|failed), `failure_reason`, `executed_at`; UNIQUE (`run_id`, `item_id`) |
| **ap_tax_snapshots** | Tax lifecycle per bill | `id` (UUID), `bill_id` (FK), `tenant_id`, `provider`, `provider_quote_ref`, `provider_commit_ref`, `quote_hash`, `total_tax_minor`, `tax_by_line` (JSONB), `status` (quoted\|committed\|voided), `quoted_at`, `committed_at` (nullable), `voided_at` (nullable), `void_reason` (nullable), `created_at`, `updated_at`; UNIQUE active per bill |
| **idempotency_keys** | HTTP request idempotency | `id` (BIGSERIAL), `tenant_id`, `idempotency_key`, `request_hash`, `response_body` (JSONB), `status_code`, `expires_at`; UNIQUE (`tenant_id`, `idempotency_key`) |
| **events_outbox** | Standard platform outbox | Module-owned, same schema as other modules |
| **processed_events** | Event deduplication | Module-owned, same schema as other modules |

**Monetary Precision:** All monetary amounts use **integer minor units** (e.g., `total_minor` in cents). Currency stored as 3-letter ISO 4217 code (CHAR 3).

**Tenant Isolation:** Primary tables include `tenant_id` as a non-nullable field. Child tables inherit tenant scope via FKs. Unique constraints include `tenant_id` where relevant.

### Data NOT Owned by AP

AP **MUST NOT** store:
- Inventory stock quantities, lot/serial tracking, or warehouse locations
- GL account balances, journal entries, or chart of accounts definitions
- FX exchange rates (AP references GL fx_rates by UUID only)
- Customer invoices, receivables, or collection status
- Bank account credentials, routing numbers, or sensitive payment processor tokens
- Party-master records or cross-module identity resolution logic

---

## Bill Status Machine

```
open ──→ matched ──→ approved ──→ partially_paid ──→ paid
  │                    │              │
  │                    │              └──→ voided
  │                    └──→ voided
  ├──→ approved ──→ ...
  └──→ voided
```

### Transition Rules

| From | Allowed To | Guard |
|------|-----------|-------|
| open | approved, matched, voided | approved requires match policy check or override_reason |
| matched | approved, voided | — |
| approved | partially_paid, paid, voided | partially_paid/paid driven by allocation sum vs total |
| partially_paid | paid, voided | paid when allocations >= total_minor |
| paid | *(terminal)* | No further transitions |
| voided | *(terminal)* | Emits REVERSAL event; void_reason required |

---

## PO Status Machine

```
draft ──→ approved ──→ closed
  │          │
  └──→ cancelled ←──┘
```

### Transition Rules

| From | Allowed To | Guard |
|------|-----------|-------|
| draft | approved, cancelled | approved requires approved_by |
| approved | closed, cancelled | — |
| closed | *(terminal)* | No further transitions |
| cancelled | *(terminal)* | No further transitions |

---

## Payment Run Lifecycle

```
pending ──→ executing ──→ completed
                │
                └──→ failed
```

The builder selects eligible bills (approved / partially_paid with open balance > 0) grouped by vendor. Execution creates allocation records, transitions bill statuses, and emits per-vendor `ap.payment_executed` events.

---

## Events Produced

All events use the platform `EventEnvelope` and are written to the module outbox atomically with the triggering mutation.

| Event | Trigger | Mutation Class | Key Payload Fields |
|-------|---------|---------------|-------------------|
| `ap.vendor_created` | Vendor registered | DATA_MUTATION | `vendor_id`, `tenant_id`, `name`, `currency`, `payment_terms_days`, `payment_method` |
| `ap.vendor_updated` | Vendor attributes changed or deactivated | DATA_MUTATION | `vendor_id`, `tenant_id`, changed fields (None = unchanged), `updated_by` |
| `ap.po_created` | Purchase order created | DATA_MUTATION | `po_id`, `tenant_id`, `vendor_id`, `po_number`, `currency`, `lines[]`, `total_minor` |
| `ap.po_approved` | Purchase order approved | DATA_MUTATION | `po_id`, `vendor_id`, `po_number`, `approved_amount_minor`, `approved_by` |
| `ap.po_closed` | PO fully received or manually closed | LIFECYCLE | `po_id`, `vendor_id`, `po_number`, `close_reason` (fully_received\|cancelled\|manual_close) |
| `ap.po_line_received_linked` | PO line linked to a goods receipt | DATA_MUTATION | `po_id`, `po_line_id`, `vendor_id`, `receipt_id`, `quantity_received`, `unit_price_minor`, `gl_account_code` |
| `ap.vendor_bill_created` | Vendor bill entered | DATA_MUTATION | `bill_id`, `vendor_id`, `vendor_invoice_ref`, `currency`, `lines[]`, `total_minor`, `due_date` |
| `ap.vendor_bill_matched` | Bill matched to PO via match engine | DATA_MUTATION | `bill_id`, `po_id`, `match_type`, `match_lines[]`, `fully_matched` |
| `ap.vendor_bill_approved` | Bill approved for payment | DATA_MUTATION | `bill_id`, `vendor_id`, `approved_amount_minor`, `currency`, `due_date`, `fx_rate_id`, `gl_lines[]` |
| `ap.vendor_bill_voided` | Bill voided (compensating event) | REVERSAL | `bill_id`, `vendor_id`, `original_total_minor`, `void_reason`, `reverses_event_id` |
| `ap.payment_run_created` | Payment run batch created | DATA_MUTATION | `run_id`, `items[]` (per-vendor), `total_minor`, `currency`, `payment_method` |
| `ap.payment_executed` | Per-vendor payment disbursed | DATA_MUTATION | `payment_id`, `run_id`, `vendor_id`, `bill_ids[]`, `amount_minor`, `payment_method`, `bank_reference` |

---

## Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| `inventory.item_received` | Inventory module | Ingests receipt link into `po_receipt_links` for 3-way match. Single-line POs auto-inferred; multi-line POs require explicit receipt link API. Idempotent via UNIQUE (`po_line_id`, `receipt_id`). |

---

## Integration Points

### Inventory (Event Consumer, One-Way)

AP subscribes to `inventory.item_received` via NATS. The consumer looks up the PO in AP's own database (no cross-module read), infers the PO line for single-line POs, and creates a receipt link in `po_receipt_links`. This link anchors the 3-way match. **Multi-line POs:** auto-inference is not attempted; logged as warning, requires explicit API call.

### GL (Event-Driven, One-Way)

`ap.vendor_bill_approved` carries `gl_lines[]` with per-line `gl_account_code`, `amount_minor`, and `po_line_id` (for inventory clearing routing). `ap.vendor_bill_voided` carries `reverses_event_id` for GL reversal. `ap.payment_executed` enables payment clearing entries. **AP never calls GL.** GL subscribes to the events.

### Payments (Integration Seam, Synchronous)

The `integrations::payments` module is the seam between AP payment runs and the disbursement service. Currently a local implementation with deterministic UUID v5 payment IDs. **Integration note:** the function body is designed to be replaced with an HTTP call to the Payments disbursement service when available.

### Tax (Shared Trait, In-Process)

AP uses the `tax-core` crate's `TaxProvider` trait for tax quote / commit / void lifecycle. The default `ZeroTaxProvider` returns zero tax for all requests. External tax providers (e.g., Avalara) implement the same trait. Tax snapshots are persisted per bill in `ap_tax_snapshots`.

### Party-Master (Optional, Reference Only)

`vendors.party_id` optionally links to a party-master UUID. Set at vendor creation time. Never queried at runtime. Enables cross-module identity correlation (e.g., a party who is both a vendor and a customer).

### Security (Platform Crate, In-Process)

JWT verification via the platform `security` crate. Write operations require `AP_MUTATE` permission. Actor identity (`actor_id`, `actor_type`) from verified claims is propagated into event envelopes.

---

## Invariants

1. **Tenant isolation is unbreakable.** Primary tables carry `tenant_id`; child tables inherit scope via FKs. Every query filters by `tenant_id`. Unique constraints include `tenant_id` where relevant. No cross-tenant data leakage.
2. **Allocations are append-only.** No UPDATE or DELETE on `ap_allocations`. Financial history is immutable.
3. **Outbox atomicity.** Every state-changing mutation writes its event to the outbox in the same database transaction. No silent event loss.
4. **Bill status derives from allocations.** `partially_paid` vs `paid` is computed deterministically from `SUM(allocations) vs total_minor`. No manual status override for payment state.
5. **Vendor invoice uniqueness per tenant.** `(tenant_id, vendor_id, vendor_invoice_ref)` is unique — prevents duplicate bill entry.
6. **PO number uniqueness per tenant.** `(tenant_id, po_number)` is unique.
7. **Match record uniqueness per bill line.** One match record per `bill_line_id` — re-running the engine is idempotent.
8. **Receipt link uniqueness.** `(po_line_id, receipt_id)` is unique — replaying `inventory.item_received` events is safe.
9. **Payment run execution uniqueness.** `(run_id, item_id)` is unique — re-executing a payment run does not create duplicate disbursements.
10. **Payment ID determinism.** UUID v5 from `run_id:vendor_id` — same inputs always produce the same payment identifier.
11. **All events are replay-safe.** Every event payload is self-contained. Downstream consumers never need to read AP state to process events correctly.
12. **Void is compensating, not destructive.** Voided bills remain in the database. REVERSAL events carry `reverses_event_id` for GL correlation.
13. **Active vendor name uniqueness per tenant.** Only one active vendor per `(tenant_id, name)` — enforced by partial unique index.
14. **At most one active tax snapshot per bill.** Partial unique index on `(bill_id) WHERE status != 'voided'`.

---

## API Surface (Summary)

### Operational
- `GET /healthz` — Liveness probe
- `GET /api/health` — Legacy health check
- `GET /api/ready` — Readiness probe (verifies DB connectivity)
- `GET /api/version` — Module identity and schema version
- `GET /metrics` — Prometheus metrics

### Vendors
- `POST /api/ap/vendors` — Create vendor
- `GET /api/ap/vendors` — List vendors (tenant-scoped, optional inactive filter)
- `GET /api/ap/vendors/{vendor_id}` — Get vendor detail
- `PUT /api/ap/vendors/{vendor_id}` — Update vendor
- `POST /api/ap/vendors/{vendor_id}/deactivate` — Soft-deactivate vendor

### Purchase Orders
- `POST /api/ap/pos` — Create PO (always draft)
- `GET /api/ap/pos` — List POs (tenant-scoped)
- `GET /api/ap/pos/{po_id}` — Get PO with lines
- `PUT /api/ap/pos/{po_id}/lines` — Replace all PO lines (draft only)
- `POST /api/ap/pos/{po_id}/approve` — Approve PO

### Bills
- `POST /api/ap/bills` — Create vendor bill with lines
- `GET /api/ap/bills` — List bills (tenant-scoped)
- `GET /api/ap/bills/{bill_id}` — Get bill with lines
- `POST /api/ap/bills/{bill_id}/match` — Run match engine against a PO
- `POST /api/ap/bills/{bill_id}/approve` — Approve bill for payment
- `POST /api/ap/bills/{bill_id}/void` — Void bill (compensating event)
- `POST /api/ap/bills/{bill_id}/tax-quote` — Quote tax for a bill

### Allocations
- `POST /api/ap/bills/{bill_id}/allocations` — Apply payment allocation
- `GET /api/ap/bills/{bill_id}/allocations` — List allocations for a bill
- `GET /api/ap/bills/{bill_id}/balance` — Get open balance summary

### Payment Runs
- `POST /api/ap/payment-runs` — Create payment run
- `GET /api/ap/payment-runs/{run_id}` — Get payment run detail
- `POST /api/ap/payment-runs/{run_id}/execute` — Execute payment run

### Reports
- `GET /api/ap/aging` — AP aging report (by currency, optional vendor breakdown)
- `GET /api/ap/tax/reports/summary` — Tax summary report
- `GET /api/ap/tax/reports/export` — Tax report export

### Admin (requires `X-Admin-Token` header)
- `POST /api/ap/admin/projection-status` — Query projection processing status
- `POST /api/ap/admin/consistency-check` — Run projection consistency check
- `GET /api/ap/admin/projections` — List all projections

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`

---

## Decision Log

Every significant product, architecture, or standards decision is recorded here. Do not re-open a decision without adding a new row that supersedes the old one.

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-18 | Vendor register is AP-owned with optional party-master linkage | AP needs payment-relevant vendor data (terms, currency, method) without depending on an external party-master service; party_id is informational only | Platform Orchestrator |
| 2026-02-18 | POs are commitments, not accounting events | AP liability is only created at bill entry, not PO creation; keeps GL clean and avoids premature accrual entries | Platform Orchestrator |
| 2026-02-18 | Allocations are append-only (no UPDATE, no DELETE) | Financial payment history must be fully auditable; immutable records prevent retrospective tampering | Platform Orchestrator |
| 2026-02-18 | 3-way match engine is deterministic and non-blocking | Match results are computed and stored but do not prevent bill processing; approval override with documented reason provides flexibility | Platform Orchestrator |
| 2026-02-18 | Bill voiding uses compensating REVERSAL events | Bills are never deleted; REVERSAL mutation class with reverses_event_id enables GL to post reversing entries autonomously | Platform Orchestrator |
| 2026-02-18 | Payment ID is deterministic via UUID v5 | run_id + vendor_id deterministically produces the same payment_id on retries, making the Payments integration naturally idempotent without external round-trips | Platform Orchestrator |
| 2026-02-18 | Bill due date derived from invoice_date + payment_terms_days | Pure function with deterministic output; explicit due_date override available when vendor terms differ from default | Platform Orchestrator |
| 2026-02-18 | Match policy requires override_reason for unmatched or out-of-tolerance approvals | Enforces accountability — approving without matching is allowed but requires documented justification | Platform Orchestrator |
| 2026-02-18 | Vendor bill status machine separates open/matched/approved from partially_paid/paid | Financial status (payment state) derives from allocation sums, not manual updates; approval is a separate gate from payment | Platform Orchestrator |
| 2026-02-18 | PO status changes are logged to po_status (append-only audit trail) | Enables audit queries on who approved, when, and why; denormalized status on purchase_orders is for query performance | Platform Orchestrator |
| 2026-02-18 | Single-line PO receipt link auto-inferred from inventory events; multi-line POs require explicit API | Avoids ambiguity — when there are multiple lines, only a human or business rule can determine which line the receipt applies to | Platform Orchestrator |
| 2026-02-18 | FX rate reference stored as UUID pointer to GL fx_rates, not as a raw rate | Reuses existing GL FX infrastructure; prevents rate duplication and ensures single source of truth for conversion | Platform Orchestrator |
| 2026-02-18 | ap.vendor_bill_approved carries gl_lines[] for replay-safe GL posting | GL consumer has all per-line expense account routing without re-reading AP database; po_line_id presence determines clearing vs expense posting | Platform Orchestrator |
| 2026-02-19 | party_id added to vendors as nullable column with no FK constraint | Party-master lives in a separate service; loose coupling via UUID reference enables cross-module identity without runtime dependency | Platform Orchestrator |
| 2026-02-18 | Tenant isolation via tenant_id on primary tables with partial unique indexes | Standard platform multi-tenant pattern; child tables inherit tenant scope via FKs; unique constraints include tenant_id where business uniqueness is per-tenant | Platform Orchestrator |
| 2026-02-18 | No mocking in tests — integrated tests against real Postgres | Platform-wide standard; mocked tests provide false confidence; all verification hits real database | Platform Orchestrator |
