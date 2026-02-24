# AR (Accounts Receivable) Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v1.0.1)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | DarkOwl | Initial vision doc — documented from existing codebase (v1.0.1). Business problem, user personas, design principles, structural decisions, full data ownership, events, API surface, invariants, decision log. |
| 1.1 | 2026-02-24 | CopperRiver | Fresh-eyes review (bd-3i6y). Fixed state machine (removed unimplemented DRAFT→OPEN, added UNCOLLECTIBLE note). Removed invented `ar.payment.received` event. Added missing tax lifecycle API routes. Added `ar_tax_rules` and `ar_invoice_tax_snapshots` tables. Added `payment_intent` to Tilled client list. |

---

## The Business Problem

Every business that bills customers has the same fundamental challenge: **getting paid reliably, on time, and with full visibility into what's owed.**

Small and mid-size service businesses — waste haulers, property managers, field service companies — often manage invoicing through spreadsheets, disconnected payment processors, or manual bookkeeping. They can't answer basic questions: Which customers owe us money? How old is the debt? Did that payment actually land? When a customer disputes a charge, was the original invoice correct?

The gap between "we sent an invoice" and "we received payment" is where money disappears. Payments fail silently, disputes go unnoticed, delinquent accounts aren't followed up, and the accounting team reconstructs the truth at month-end from bank statements. By then it's too late — the revenue was already lost.

A modern AR system must do more than issue invoices. It must track the full lifecycle — from charge creation through payment collection, allocation, aging, dunning, reconciliation, and if necessary, write-off — with every step auditable, idempotent, and integrated into the general ledger.

---

## What the Module Does

The AR module is the **authoritative system for customer receivables, invoicing, payment processing, and revenue recognition events** across the platform. It is the financial backbone that answers:

1. **Who owes us what?** — Customer accounts with subscription and one-time charges, invoices with line items, and real-time aging buckets (current, 1-30, 31-60, 61-90, 90+ days overdue).
2. **Did the payment succeed?** — Integration with Tilled (payment processor) via webhook ingestion, HMAC-verified, with idempotent deduplication by `event_id`.
3. **How do payments match to invoices?** — FIFO payment allocation, automated reconciliation runs with deterministic matching, and exception raising for unmatched items.
4. **What happens when they don't pay?** — Dunning state machine (pending → warned → escalated → suspended), exponential backoff retry scheduling, and configurable escalation paths.
5. **How does this flow to accounting?** — GL journal entry emission via NATS events, tax quote/commit/void lifecycle, credit note issuance, and invoice write-off as formal financial reversals.
6. **What about metered billing?** — Usage capture with idempotent ingestion, usage-to-invoice conversion with exactly-once billing guarantees.

---

## Who Uses This

The module is a platform service consumed by vertical applications (TrashTech, etc.) and the Tenant Control Plane. It does not have its own frontend — it exposes an API that frontends consume.

### Billing Administrator
- Creates and manages customer accounts
- Defines subscription plans with pricing and intervals
- Issues one-time charges for ad-hoc services
- Reviews aging reports and outstanding balances
- Authorizes credit notes and write-offs
- Configures tax jurisdictions and rules

### Operations / Collections
- Monitors dunning status across delinquent accounts
- Triggers reconciliation runs to match payments to invoices
- Reviews reconciliation exceptions (unmatched, overpaid, underpaid)
- Manages payment allocation when automatic matching fails
- Escalates suspended accounts

### Finance / Accounting
- Receives GL posting events for journal entry creation
- Reviews tax reports (summary and export)
- Audits credit notes and write-offs for compliance
- Correlates AR aging with cash flow forecasts (consumed by cash flow module)

### System (Payment Processor Integration)
- Receives Tilled webhooks for payment success/failure events
- Verifies webhook signatures (HMAC-SHA256)
- Deduplicates events by `event_id` to prevent double-processing
- Routes payment outcomes to invoice status transitions

### System (Background Workers)
- Dunning scheduler: polls for due dunning rows, executes collection attempts with exponential backoff
- Reconciliation scheduler: claims and executes reconciliation runs for time windows
- Outbox publisher: polls events_outbox and publishes to NATS
- Payment succeeded consumer: subscribes to `payments.events.payment.succeeded`, marks invoices paid

---

## Design Principles

### Exactly-Once Everywhere
Every mutation that produces side effects uses deterministic idempotency keys, `ON CONFLICT DO NOTHING`, and outbox atomicity (mutation + event in same transaction). Replay safety is a first-class invariant, not an afterthought.

### Guard → Mutate → Emit
All state-changing operations follow a three-phase pattern: (1) a pure guard validates the transition with zero side effects, (2) the mutation is applied, (3) events are emitted — all within a single database transaction. This prevents partial state and ensures every mutation either fully commits or fully rolls back.

### Invoice Lifecycle Is a State Machine
Invoice status transitions (open→attempting, attempting→paid/failed_final, open→void) are enforced by a domain state machine in `src/lifecycle.rs`. No direct SQL updates to status columns. Every transition is validated, logged, and event-emitted. Status constants for `draft` and `uncollectible` exist but have no guarded transitions in v1.0.1.

### Financial Artifacts Are Append-Only
Credit notes, write-offs, and reconciliation matches are never updated or deleted. Corrections require new compensating entries. This provides a complete audit trail and makes GL integration reliable.

### Payment Processor Agnostic (via Tilled)
The module integrates with Tilled as its payment processor, but the domain logic (invoices, charges, subscriptions, dunning) is decoupled from Tilled-specific types. Tilled webhook events are translated into domain events at the boundary.

### Standalone First
The module boots and runs without Payments, GL, Notifications, or Party Master. Each integration degrades gracefully — Party Master validation is optional, GL events are fire-and-forget (outbox + NATS), and dunning operates independently of notification delivery.

---

## Current Scope (v1.0.1)

### Built and Proven
- Customer lifecycle (create/update/suspend/reactivate, status-gated operations)
- Subscription management (create/update/cancel with period tracking)
- Invoice lifecycle (open → attempting → paid/failed_final, open → void), aging buckets (0-30/31-60/61-90/90+)
- Invoice attempt ledger with exactly-once finalization gating
- Fixed retry windows (attempt 0: immediate, +3 days, +7 days, max 3 attempts)
- Charge management (one-time and recurring, authorize/capture flow)
- Refund processing
- Dispute tracking with evidence submission
- Payment method management (card and bank, default selection)
- Credit note issuance (append-only, idempotent by `credit_note_id`)
- Invoice write-off / bad debt (append-only, one per invoice, REVERSAL mutation class)
- Tilled webhook ingestion (HMAC-SHA256 verification, `event_id` deduplication)
- Payment allocation (FIFO strategy, idempotent by `idempotency_key`)
- AR aging projection (computed and stored per customer/currency, event emitted on refresh)
- Dunning state machine (pending → warned → escalated → suspended → resolved/written_off)
- Dunning scheduler worker (FOR UPDATE SKIP LOCKED, bounded exponential backoff)
- Reconciliation engine (deterministic matching, exception raising, scheduled runs)
- Metered usage capture (idempotent ingestion) and usage-to-invoice billing (exactly-once via `billed_at` sentinel)
- Tax quote/commit/void lifecycle with cached quotes and provider abstraction
- Tax jurisdiction and rule configuration
- Tax reporting (summary and export)
- FX settlement gain/loss event emission
- GL journal entry emission via NATS events
- Invoice lifecycle events (`ar.invoice_opened`, `ar.invoice_paid`) for cash flow forecasting
- Payment succeeded consumer (subscribes to Payments module events)
- Outbox/inbox pattern with dead-letter queue for failed events
- Party Master cross-module validation (optional party_id on customers, invoices, subscriptions)
- Prometheus metrics (invoices created/paid, request latency, error rate, consumer lag)
- Coupon and discount management (plan-level and charge-level)
- Plans and addons catalog

### Explicitly Out of Scope
- Multi-currency settlement (FX events are emitted but no active FX engine)
- Active Payments module integration for collection orchestration (currently via event consumption only)
- Automated refund processing through Tilled API (refunds tracked but not auto-submitted)
- Subscription proration on plan changes
- Revenue recognition rules (deferred revenue, ASC 606 compliance)
- Frontend UI (consumed via API by vertical apps or TCP)

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum | Port 8086 (default) |
| Database | PostgreSQL | Dedicated database, SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate, configurable (NATS or in-memory) |
| Auth | JWT via platform `security` crate | Permission-gated mutations (`ar.mutate`), rate-limited webhooks |
| Outbox | Platform outbox pattern | `events_outbox` table, background publisher task |
| Payment processor | Tilled | SDK-less HTTP client, webhook-driven, HMAC-SHA256 verification |
| Tax | Pluggable `TaxProvider` trait | Local provider built-in, extensible to Avalara/TaxJar |
| Metrics | Prometheus | `/metrics` endpoint, SLO histograms |
| Projections | Platform `projections` crate | Used for aging bucket computation |
| Crate | `ar-rs` | Single crate, modular source layout |

---

## Structural Decisions (The "Walls")

### 1. Tilled as the single payment processor — no PSP abstraction layer
The module integrates directly with Tilled via HTTP client and webhook verification. There is no generic "payment service provider" abstraction because the business uses Tilled exclusively. The integration surface is contained in `src/tilled/` with per-entity clients (customer, payment_method, payment_intent, subscription, refund, dispute, webhook). If a second PSP is ever needed, the Tilled-specific code is isolated and can be wrapped.

### 2. Invoice lifecycle enforced by domain state machine
All invoice status transitions go through `src/lifecycle.rs`, which validates allowed transitions (open→attempting, attempting→paid, attempting→failed_final, open→void). Direct SQL updates to `ar_invoices.status` are forbidden. This guarantees every transition is logged, event-emitted, and auditable. Note: `draft` and `uncollectible` exist as status constants (used in query filters and as initial insert values) but have no guarded transitions in the state machine.

### 3. Exactly-once finalization via SELECT FOR UPDATE + attempt ledger
Invoice finalization uses `SELECT FOR UPDATE` to lock the invoice row, then inserts into `ar_invoice_attempts` with a `UNIQUE(app_id, invoice_id, attempt_no)` constraint. Duplicate attempts return `AlreadyProcessed` (deterministic no-op). Side effects only fire when the attempt row is newly created. This prevents double-charging under concurrent requests.

### 4. Credit notes and write-offs are append-only compensating entries
Credit notes reduce invoice balance without modifying the original invoice. Write-offs forgive uncollectable debt as a formal REVERSAL. Neither is ever updated or deleted — corrections require new entries. This matches double-entry accounting principles and makes GL integration reliable.

### 5. Dunning is a deterministic state machine with optimistic locking
Each `(app_id, invoice_id)` has exactly one dunning record. Transitions use a monotonic `version` field for compare-and-swap safety. Terminal states (resolved, written_off) have no further transitions. The scheduler uses `FOR UPDATE SKIP LOCKED` for concurrent worker safety.

### 6. Payment allocation is FIFO with explicit allocation rows
Payments are allocated to invoices oldest-due-first. Each allocation is a row in `ar_payment_allocations` with an `idempotency_key` for replay safety. Allocations are never modified — overpayment remains as unallocated balance. This provides a complete audit trail of how every dollar was applied.

### 7. Reconciliation matches are immutable
Recon runs produce matches and exceptions as append-only rows. Match decisions are never retroactively changed. If a match was wrong, a new recon run produces new results. This preserves the full history of how the system's understanding evolved.

### 8. Multi-tenant isolation via app_id on every table
Standard platform multi-tenant pattern. Every table has `app_id` as a non-nullable field. Every query filters by `app_id`. Every index includes `app_id` as a leading column. Cross-tenant data leakage is impossible at the query level.

### 9. Webhook ingestion is rate-limited and signature-verified separately from API auth
Tilled webhooks hit a dedicated endpoint (`/api/ar/webhooks/tilled`) with IP-based rate limiting but no JWT auth (webhooks come from Tilled, not authenticated users). HMAC-SHA256 signature verification happens at the application layer. This separation prevents webhook processing from being blocked by auth failures.

### 10. Tax lifecycle is quote → commit → void with cached quotes
Tax quotes are cached in `ar_tax_quote_cache` keyed by `(app_id, invoice_id, idempotency_key)`. The same request hash always returns the same tax — no re-computation on replay. Tax is committed when an invoice is finalized and voided when an invoice is voided or written off. The `TaxProvider` trait enables swapping providers without changing domain logic.

---

## Domain Authority

AR is the **source of truth** for:

| Domain Entity | AR Authority |
|---------------|-------------|
| **Customers** | Customer accounts with email, name, external ID, Tilled customer ID, status (active/suspended), delinquency tracking, optional Party Master link. |
| **Subscriptions** | Recurring billing agreements: plan, price, interval, period tracking, cancellation state. Tilled subscription reference. |
| **Invoices** | Receivables with status lifecycle (open → attempting → paid/failed_final, open → void), amount, currency, due date, billing period, line items, compliance codes. |
| **Invoice Attempts** | Deterministic attempt ledger per invoice: attempt number, status, idempotency key. Enforces exactly-once finalization. |
| **Charges** | One-time and recurring charges: amount, type, service date, product details, location reference. Linked to invoices and customers. |
| **Refunds** | Refunds against charges: amount, status, Tilled refund reference. |
| **Disputes** | Payment disputes with evidence tracking, reason codes, and resolution deadlines. |
| **Payment Methods** | Customer payment instruments: card (brand, last4, expiry) or bank (name, last4). Default selection per customer. |
| **Credit Notes** | Formal compensating entries against invoices: amount, reason, append-only. |
| **Write-offs** | Bad debt forgiveness records: amount, reason, authorized_by, one per invoice, append-only. |
| **Aging Buckets** | Pre-computed AR aging snapshots per customer/currency: current, 1-30, 31-60, 61-90, 90+ days overdue. |
| **Dunning State** | Collection escalation state per invoice: state machine, attempt count, next retry time, optimistic locking version. |
| **Reconciliation** | Recon runs, matches, and exceptions: deterministic matching of payments to invoices with full audit trail. |
| **Payment Allocations** | Explicit payment-to-invoice allocation rows: FIFO strategy, idempotent, amount tracking. |
| **Metered Usage** | Usage records: metric, quantity, unit price, period, billing status (`billed_at` sentinel). |
| **Tax Quotes & Commits** | Cached tax calculations and committed tax records: provider references, amounts, void tracking. |
| **Webhooks** | Inbound payment processor events: status, payload, attempt tracking, dead-letter handling. |
| **Plans & Addons** | Subscription plan catalog and add-on products with pricing. |
| **Coupons & Discounts** | Discount rules: percentage/fixed, duration, redemption limits, seasonal windows, volume tiers, stacking. |

AR is **NOT** authoritative for:
- Payment execution and processor communication (Payments module owns actual charge execution)
- GL account balances or journal entries (GL module owns the ledger)
- Customer identity, contact details, or organizational hierarchy (Party Master owns party data)
- Notification delivery (Notifications module subscribes to AR events)
- Cash flow forecasts and probability models (Cash Flow module consumes AR lifecycle events)

---

## Data Ownership

### Tables Owned by AR

All tables use `app_id` for multi-tenant isolation. Every query **MUST** filter by `app_id`.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **ar_customers** | Customer accounts | `id`, `app_id`, `external_customer_id`, `tilled_customer_id`, `status`, `email`, `name`, `default_payment_method_id`, `delinquent_since`, `grace_period_end`, `next_retry_at`, `retry_attempt_count`, `party_id` (nullable UUID) |
| **ar_subscriptions** | Recurring billing | `id`, `app_id`, `ar_customer_id`, `tilled_subscription_id`, `plan_id`, `plan_name`, `price_cents`, `status` (enum), `interval_unit` (enum), `current_period_start/end`, `cancel_at_period_end`, `party_id` |
| **ar_invoices** | Receivables | `id`, `app_id`, `tilled_invoice_id`, `ar_customer_id`, `subscription_id`, `status`, `amount_cents`, `currency`, `due_at`, `paid_at`, `billing_period_start/end`, `line_item_details` (JSONB), `compliance_codes` (JSONB), `correlation_id`, `party_id` |
| **ar_invoice_attempts** | Finalization ledger | `id` (UUID), `app_id`, `invoice_id`, `attempt_no`, `status` (enum: attempting/succeeded/failed_retry/failed_final), `idempotency_key`, UNIQUE(app_id, invoice_id, attempt_no) |
| **ar_charges** | One-time/recurring charges | `id`, `app_id`, `tilled_charge_id`, `invoice_id`, `ar_customer_id`, `status`, `amount_cents`, `currency`, `charge_type`, `reason`, `reference_id`, `service_date`, `product_type`, `quantity` |
| **ar_refunds** | Refunds against charges | `id`, `app_id`, `ar_customer_id`, `charge_id`, `tilled_refund_id`, `status`, `amount_cents`, `currency`, `reason`, `reference_id` |
| **ar_disputes** | Payment disputes | `id`, `app_id`, `tilled_dispute_id`, `charge_id`, `status`, `amount_cents`, `reason`, `evidence_due_by` |
| **ar_payment_methods** | Customer payment instruments | `id`, `app_id`, `ar_customer_id`, `tilled_payment_method_id`, `status`, `type`, `brand`, `last4`, `exp_month/year`, `is_default` |
| **ar_credit_notes** | Compensating credits | `credit_note_id` (UUID, business key), `app_id`, `customer_id`, `invoice_id`, `amount_minor` (BIGINT), `currency`, `reason`, `status` (issued, append-only) |
| **ar_invoice_write_offs** | Bad debt forgiveness | `write_off_id` (UUID, business key), `app_id`, `invoice_id`, `customer_id`, `written_off_amount_minor`, `currency`, `reason`, UNIQUE(invoice_id) |
| **ar_dunning_states** | Collection escalation | `dunning_id` (UUID), `app_id`, `invoice_id`, `customer_id`, `state`, `version` (optimistic lock), `attempt_count`, `next_attempt_at`, UNIQUE(app_id, invoice_id) |
| **ar_aging_buckets** | Aging projection | `app_id`, `customer_id`, `currency`, `current_minor`, `days_1_30_minor`, `days_31_60_minor`, `days_61_90_minor`, `days_over_90_minor`, `total_outstanding_minor`, `invoice_count`, UNIQUE(app_id, customer_id, currency) |
| **ar_payment_allocations** | Payment-to-invoice mapping | `app_id`, `payment_id`, `invoice_id`, `amount_cents`, `strategy` (fifo), `idempotency_key`, UNIQUE(idempotency_key) |
| **ar_recon_runs** | Reconciliation runs | `recon_run_id` (UUID), `app_id`, `status`, `matching_strategy`, `payment_count`, `invoice_count`, `match_count`, `exception_count` |
| **ar_recon_matches** | Recon match decisions (append-only) | `match_id` (UUID), `recon_run_id`, `app_id`, `payment_id`, `invoice_id`, `matched_amount_minor`, `confidence_score`, `match_method` |
| **ar_recon_exceptions** | Recon exceptions (append-only) | `exception_id` (UUID), `recon_run_id`, `app_id`, `payment_id`, `invoice_id`, `exception_kind`, `description` |
| **ar_recon_scheduled_runs** | Scheduled recon windows | `scheduled_run_id` (UUID), `app_id`, `window_start`, `window_end`, `status`, UNIQUE(app_id, window_start, window_end) |
| **ar_metered_usage** | Usage records | `app_id`, `customer_id`, `subscription_id`, `metric_name`, `quantity`, `unit_price_cents`, `period_start/end`, `billed_at` (sentinel) |
| **ar_invoice_line_items** | Invoice line items | `app_id`, `invoice_id`, `line_item_type`, `description`, `quantity`, `unit_price_cents`, `amount_cents` |
| **ar_plans** | Subscription plan catalog | `app_id`, `plan_id`, `name`, `interval_unit`, `price_cents`, `currency`, `features` (JSONB), `active` |
| **ar_addons** | Add-on products | `app_id`, `addon_id`, `name`, `price_cents`, `currency`, `features` (JSONB) |
| **ar_subscription_addons** | Subscription↔addon junction | `subscription_id`, `addon_id`, `quantity` |
| **ar_coupons** | Discount rules | `app_id`, `code`, `coupon_type`, `value`, `duration`, `max_redemptions`, `stackable`, `priority`, `volume_tiers` (JSONB) |
| **ar_discount_applications** | Applied discounts | `app_id`, `invoice_id`, `charge_id`, `coupon_id`, `customer_id`, `discount_amount_cents` |
| **ar_tax_rates** | Tax rate definitions | `app_id`, `jurisdiction_code`, `tax_type`, `rate`, `effective_date`, UNIQUE(app_id, jurisdiction_code, tax_type, effective_date) |
| **ar_tax_calculations** | Applied tax per invoice/charge | `app_id`, `invoice_id`, `charge_id`, `tax_rate_id`, `taxable_amount_cents`, `tax_amount_cents`, `rate_applied` |
| **ar_tax_quote_cache** | Cached tax quotes | `app_id`, `invoice_id`, `idempotency_key`, `provider_quote_ref`, `total_tax_minor`, `tax_by_line` (JSONB), UNIQUE(app_id, invoice_id, idempotency_key) |
| **ar_tax_commits** | Tax commit/void ledger | `app_id`, `invoice_id`, `provider_commit_ref`, `total_tax_minor`, `status` (committed/voided), UNIQUE(app_id, invoice_id) |
| **ar_tax_jurisdictions** | Tax jurisdiction config | `app_id`, per-jurisdiction tax configuration |
| **ar_tax_rules** | Tax rate rules per jurisdiction | `app_id`, `jurisdiction_id`, `tax_code`, `rate` (NUMERIC), `flat_amount_minor`, `effective_from`, `effective_to` |
| **ar_invoice_tax_snapshots** | Resolved jurisdiction snapshot per invoice | `app_id`, `invoice_id`, `jurisdiction_id`, `jurisdiction_name`, `country_code`, `state_code`, `tax_code`, `rate_applied`, `taxable_minor`, `tax_minor` |
| **ar_webhooks** | Inbound webhook log | `app_id`, `event_id`, `event_type`, `status` (enum), `payload` (JSONB), `attempt_count`, UNIQUE(event_id, app_id) |
| **ar_webhook_attempts** | Webhook processing attempts | `app_id`, `event_id`, `attempt_number`, `status`, `error_code` |
| **ar_dunning_config** | Per-tenant dunning settings | `app_id` (UNIQUE), `grace_period_days`, `retry_schedule_days` (JSONB), `max_retry_attempts` |
| **ar_idempotency_keys** | HTTP idempotency cache | `app_id`, `idempotency_key`, `request_hash`, `response_body` (JSONB), `expires_at`, UNIQUE(app_id, idempotency_key) |
| **ar_events** | Audit event log | `app_id`, `event_type`, `source`, `entity_type`, `entity_id`, `payload` (JSONB) |
| **ar_reconciliation_runs** | Legacy recon runs | `app_id`, `status`, `stats` (JSONB) |
| **ar_divergences** | Legacy recon divergences | `app_id`, `run_id`, `entity_type`, `divergence_type`, `local_snapshot`, `remote_snapshot` |
| **events_outbox** | Platform outbox | Standard outbox schema with envelope metadata |
| **processed_events** | Event deduplication | Standard dedup table for consumed events |
| **failed_events** | Dead letter queue | Failed event processing records |

**Monetary Precision:** All monetary amounts use **integer minor units** (e.g., `amount_cents`, `amount_minor` in cents). Currency stored as 3-letter ISO 4217 code.

### Data NOT Owned by AR

AR **MUST NOT** store:
- Payment execution state or processor transaction IDs (Payments module owns charge execution)
- GL account codes, journal entries, or account balances (GL module owns the ledger)
- Party identity data beyond the opaque `party_id` reference (Party Master owns party records)
- Notification templates, delivery status, or channel configuration (Notifications module owns delivery)
- Cash flow forecast parameters or probability models (Cash Flow module owns forecasting)

---

## Invoice State Machine

```
OPEN ──→ ATTEMPTING ──→ PAID (terminal)
  |            |
  |            └──→ FAILED_FINAL (terminal)
  |
  └──→ VOID (terminal)
```

**Note:** `draft` and `uncollectible` exist as status constants in `lifecycle.rs` and `models/invoice.rs` (used in query filters and as initial insert values) but are not wired into the `validate_transition` guard. Invoices are typically inserted as `open` directly. Future versions may add guarded transitions for these statuses.

### Transition Rules (enforced by `validate_transition` in `src/lifecycle.rs`)

| From | Allowed To | Guard |
|------|-----------|-------|
| open | attempting | Payment collection initiated (via finalization) |
| open | void | Invoice cancelled before collection |
| attempting | paid | Payment succeeded |
| attempting | failed_final | All retry attempts exhausted |
| paid | *(terminal)* | No further transitions |
| failed_final | *(terminal)* | No further transitions |
| void | *(terminal)* | No further transitions |

### Dunning State Machine

```
[Pending] ──attempt──→ [Warned] ──attempt──→ [Escalated] ──→ [Suspended]
    |                     |                       |               |
    └──paid──→ [Resolved] ←──────────────────────┘───────────────┘
    |                                             |
    └──writeoff──→ [WrittenOff] ←────────────────┘
```

Terminal states: Resolved, WrittenOff. Optimistic locking via monotonic `version` field.

---

## Events Produced

All events use the platform `EventEnvelope` and are written to the outbox atomically with the triggering mutation. Schema version: `1.0.0`.

| Event | Trigger | Mutation Class | Key Payload Fields |
|-------|---------|---------------|-------------------|
| `ar.invoice_opened` | Invoice INSERT | LIFECYCLE | `invoice_id`, `customer_id`, `app_id`, `amount_cents`, `currency`, `created_at`, `due_at` |
| `ar.invoice_paid` | Status → paid | LIFECYCLE | `invoice_id`, `customer_id`, `app_id`, `amount_cents`, `currency`, `paid_at` |
| `ar.invoice.finalizing` | Finalization attempt created | LIFECYCLE | `invoice_id`, `attempt_id`, `attempt_no`, `tenant_id` |
| `ar.usage_captured` | Metered usage recorded | DATA_MUTATION | `usage_id`, `tenant_id`, `customer_id`, `metric_name`, `quantity`, `unit`, `period_start/end` |
| `ar.usage_invoiced` | Usage billed on invoice | DATA_MUTATION | Usage → line item details |
| `ar.credit_note_issued` | Credit note issued | DATA_MUTATION | `credit_note_id`, `tenant_id`, `customer_id`, `invoice_id`, `amount_minor`, `currency`, `reason` |
| `ar.invoice_written_off` | Invoice written off | REVERSAL | `write_off_id`, `tenant_id`, `customer_id`, `invoice_id`, `amount_minor`, `reason` |
| `ar.ar_aging_updated` | Aging projection refreshed | DATA_MUTATION | `tenant_id`, `invoice_count`, aging buckets (current through 90+), `currency` |
| `ar.dunning_state_changed` | Dunning state transition | LIFECYCLE | `tenant_id`, `dunning_id`, `invoice_id`, `from_state`, `to_state`, `attempt_count` |
| `ar.invoice_suspended` | Invoice suspended | LIFECYCLE | `tenant_id`, `invoice_id`, `customer_id`, `reason` |
| `ar.recon_run_started` | Recon run initiated | DATA_MUTATION | `recon_run_id`, `tenant_id`, `payment_count`, `invoice_count`, `matching_strategy` |
| `ar.recon_match_applied` | Payment↔invoice matched | DATA_MUTATION | `match_id`, `recon_run_id`, `payment_id`, `invoice_id`, `matched_amount_minor` |
| `ar.recon_exception_raised` | Unmatched/ambiguous item | DATA_MUTATION | `exception_id`, `recon_run_id`, `exception_kind`, `payment_id`, `invoice_id` |
| `ar.payment_allocated` | Payment allocated to invoices | DATA_MUTATION | `payment_id`, `customer_id`, `total_allocated`, allocation lines |
| `tax.quoted` | Tax quote for invoice draft | DATA_MUTATION | `invoice_id`, `total_tax_minor`, `tax_by_line`, `provider_quote_ref` |
| `tax.committed` | Tax committed on finalization | DATA_MUTATION | `invoice_id`, `total_tax_minor`, `provider_commit_ref` |
| `tax.voided` | Committed tax voided | REVERSAL | `invoice_id`, `provider_commit_ref`, `void_reason` |
| `ar.invoice_settled_fx` | FX settlement gain/loss | DATA_MUTATION | `invoice_id`, FX rate and gain/loss details |
| `ar.payment.collection.requested` | Payment collection request | — | `invoice_id`, `customer_id`, `amount_minor`, `payment_method_id` |
| `gl.posting.requested` | GL journal entry request | — | `posting_date`, `currency`, `source_doc_type/id`, `lines` (account_ref, debit, credit) |

**Removed from initial draft:** `ar.invoice.created` and `ar.payment.received` were listed as "Legacy" events for GL integration. `ar.invoice.created` exists only as a test string constant (not emitted in production code). `ar.payment.received` does not exist in the codebase at all. GL integration uses `gl.posting.requested` for journal entries.

---

## Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| `payments.events.payment.succeeded` | Payments module | Mark invoice as paid, emit `ar.invoice_paid` event. Guard: only fires if status != 'paid'. Idempotent via `event_id` deduplication + `UPDATE WHERE status != 'paid'`. |

---

## Integration Points

### Tilled (Payment Processor, HTTP + Webhooks)

AR integrates with Tilled for payment processing. The `src/tilled/` module provides typed HTTP clients for customers, payment methods, payment intents, subscriptions, refunds, disputes, and webhooks. Inbound webhooks are HMAC-SHA256 verified and deduplicated by `event_id`. **Tilled is the only payment processor supported** — no PSP abstraction layer exists.

### Payments Module (Event-Driven, Bidirectional)

AR emits `ar.payment.collection.requested` when it needs a payment collected. The Payments module executes the charge and emits `payments.events.payment.succeeded` on success. AR consumes this event to mark invoices paid. **Neither module calls the other via HTTP** — all coordination is through NATS events.

### GL Module (Event-Driven, One-Way)

AR emits `gl.posting.requested` with journal entry lines (debit/credit, account refs, amounts). GL subscribes and posts the entries. **AR never calls GL.** GL subscribes to events.

### Cash Flow Module (Event-Driven, One-Way)

AR emits `ar.invoice_opened` and `ar.invoice_paid` lifecycle events. The cash flow forecasting module (Phase 51) consumes these to build probabilistic payment timing models. **AR never calls Cash Flow.** Cash Flow subscribes to events.

### Party Master (HTTP, Optional)

When `party_id` is provided on customer/invoice/subscription creation, AR validates the party exists in Party Master and belongs to the same tenant via HTTP GET. If Party Master is unavailable, AR returns 503. If `party_id` is not provided, no validation occurs. **Party validation is not required for AR to function.**

### Notifications (Event-Driven, One-Way)

The Notifications module subscribes to AR events (dunning state changes, overdue invoices, invoice suspension) to send alerts. **AR never calls Notifications.** Notifications subscribes to events.

---

## Invariants

1. **Multi-tenant isolation is unbreakable.** Every query filters by `app_id`. No cross-tenant data leakage. Every index includes `app_id` as a leading column.
2. **Invoice status transitions are guarded.** No direct SQL status updates — all transitions go through the lifecycle state machine validator. Illegal transitions are rejected and logged.
3. **Outbox atomicity.** Every state-changing mutation writes its event to the outbox in the same database transaction. No silent event loss.
4. **Exactly-once finalization.** Invoice finalization uses SELECT FOR UPDATE + UNIQUE(app_id, invoice_id, attempt_no). Duplicate attempts are deterministic no-ops.
5. **Attempt count limits.** Maximum 3 attempts per invoice (windows: 0, +3 days, +7 days). No invoice exceeds this limit.
6. **Credit notes are append-only.** Once issued, a credit note is never updated or deleted. Corrections require new credit notes.
7. **Write-offs are one-per-invoice.** UNIQUE constraint on `invoice_id` prevents double write-off. Write-offs are append-only.
8. **Reconciliation matches are immutable.** Match decisions are append-only. No retroactive changes to match rows.
9. **Webhook deduplication.** UNIQUE(event_id, app_id) on `ar_webhooks` prevents processing the same Tilled event twice.
10. **Payment allocation idempotency.** UNIQUE(idempotency_key) on `ar_payment_allocations` prevents duplicate allocation on retry.
11. **Dunning optimistic locking.** Dunning state transitions use monotonic `version` field. Concurrent transitions are detected and rejected.
12. **Tax quote determinism.** Same `(app_id, invoice_id, request_hash)` always returns the same cached tax — no re-computation on replay.
13. **No forced dependencies.** The module boots and functions without Payments, GL, Notifications, Cash Flow, or Party Master running. Every integration degrades gracefully.

---

## API Surface (Summary)

### Health & Operational
- `GET /healthz` — Liveness check
- `GET /api/health` — Health status
- `GET /api/ready` — Readiness check
- `GET /api/version` — Version info
- `GET /metrics` — Prometheus metrics

### Customers
- `POST /api/ar/customers` — Create customer
- `GET /api/ar/customers` — List customers (filterable)
- `GET /api/ar/customers/{id}` — Get customer
- `PUT /api/ar/customers/{id}` — Update customer

### Subscriptions
- `POST /api/ar/subscriptions` — Create subscription
- `GET /api/ar/subscriptions` — List subscriptions
- `GET /api/ar/subscriptions/{id}` — Get subscription
- `PUT /api/ar/subscriptions/{id}` — Update subscription
- `POST /api/ar/subscriptions/{id}/cancel` — Cancel subscription

### Invoices
- `POST /api/ar/invoices` — Create invoice
- `GET /api/ar/invoices` — List invoices (filterable by customer/subscription/status)
- `GET /api/ar/invoices/{id}` — Get invoice
- `PUT /api/ar/invoices/{id}` — Update invoice
- `POST /api/ar/invoices/{id}/finalize` — Finalize invoice (trigger collection)
- `POST /api/ar/invoices/{id}/bill-usage` — Bill metered usage onto invoice
- `POST /api/ar/invoices/{id}/credit-notes` — Issue credit note against invoice
- `POST /api/ar/invoices/{id}/write-off` — Write off invoice as bad debt

### Charges
- `POST /api/ar/charges` — Create charge
- `GET /api/ar/charges` — List charges
- `GET /api/ar/charges/{id}` — Get charge
- `POST /api/ar/charges/{id}/capture` — Capture authorized charge

### Refunds
- `POST /api/ar/refunds` — Create refund
- `GET /api/ar/refunds` — List refunds
- `GET /api/ar/refunds/{id}` — Get refund

### Disputes
- `GET /api/ar/disputes` — List disputes
- `GET /api/ar/disputes/{id}` — Get dispute
- `POST /api/ar/disputes/{id}/evidence` — Submit dispute evidence

### Payment Methods
- `POST /api/ar/payment-methods` — Add payment method
- `GET /api/ar/payment-methods` — List payment methods
- `GET /api/ar/payment-methods/{id}` — Get payment method
- `PUT /api/ar/payment-methods/{id}` — Update payment method
- `DELETE /api/ar/payment-methods/{id}` — Delete payment method
- `POST /api/ar/payment-methods/{id}/set-default` — Set default payment method

### Webhooks
- `POST /api/ar/webhooks/tilled` — Receive Tilled webhook (rate-limited, HMAC-verified)
- `GET /api/ar/webhooks` — List webhooks
- `GET /api/ar/webhooks/{id}` — Get webhook detail
- `POST /api/ar/webhooks/{id}/replay` — Replay failed webhook

### Usage
- `POST /api/ar/usage` — Capture metered usage

### Aging
- `GET /api/ar/aging` — Get aging report
- `POST /api/ar/aging/refresh` — Refresh aging projection

### Dunning
- `POST /api/ar/dunning/poll` — Poll and process due dunning rows

### Reconciliation
- `POST /api/ar/recon/run` — Execute reconciliation run
- `POST /api/ar/recon/schedule` — Create scheduled reconciliation run
- `POST /api/ar/recon/poll` — Poll and execute scheduled runs

### Payment Allocation
- `POST /api/ar/payments/allocate` — Allocate payment to invoices (FIFO)

### Tax Configuration
- `POST /api/ar/tax/config/jurisdictions` — Create tax jurisdiction
- `GET /api/ar/tax/config/jurisdictions` — List jurisdictions
- `GET /api/ar/tax/config/jurisdictions/{id}` — Get jurisdiction
- `PUT /api/ar/tax/config/jurisdictions/{id}` — Update jurisdiction
- `POST /api/ar/tax/config/rules` — Create tax rule
- `GET /api/ar/tax/config/rules` — List rules
- `GET /api/ar/tax/config/rules/{id}` — Get rule
- `PUT /api/ar/tax/config/rules/{id}` — Update rule

### Tax Lifecycle
- `POST /api/ar/tax/quote` — Request tax quote for invoice draft
- `GET /api/ar/tax/quote` — Look up cached tax quote by app_id + invoice_id
- `POST /api/ar/tax/commit` — Commit tax when invoice is finalized
- `POST /api/ar/tax/void` — Void committed tax on refund/cancellation

### Tax Reports
- `GET /api/ar/tax/reports/summary` — Tax report summary
- `GET /api/ar/tax/reports/export` — Tax report export

### Events (Audit Log)
- `GET /api/ar/events` — List events
- `GET /api/ar/events/{id}` — Get event detail

### Admin
- Admin routes (via `admin_router`) — operational management

---

## Decision Log

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-10 | Rename all tables from `billing_*` to `ar_*` prefix | Align with platform module naming convention (module abbreviation prefix). 23 tables, 3 enums, 5 FK columns renamed. 50 tests passing after migration. | Platform team |
| 2026-02-10 | Preserve `billing_` prefix on semantic domain fields | Fields like `billing_cycle_anchor`, `billing_period_start/end` describe billing domain concepts, not table references. Renaming them would lose semantic meaning. | Platform team |
| 2026-02-10 | PostgreSQL port 5436, SQLx (not Prisma) | Module uses Rust backend with Axum; SQLx is the standard Rust SQL toolkit. Port 5436 for AR-dedicated database. | Platform team |
| 2026-02-15 | Invoice attempt ledger with UNIQUE(app_id, invoice_id, attempt_no) | Prevents double-finalization under concurrent requests. Exactly-once side effects — events only fire when attempt row is newly created. UNIQUE violation → deterministic no-op. | Platform Orchestrator + ChatGPT |
| 2026-02-15 | SELECT FOR UPDATE on invoice row during finalization | Prevents concurrent finalization of the same invoice. Transaction-scoped lock released on commit/rollback. Combined with attempt ledger for belt-and-suspenders safety. | Platform Orchestrator + ChatGPT |
| 2026-02-15 | Fixed retry windows (0, +3d, +7d), max 3 attempts, no configurability | Deterministic behavior is more debuggable than configurable backoff. Three attempts with increasing delay covers most payment failure scenarios. Hard-coded to prevent misconfiguration. | Platform Orchestrator + ChatGPT |
| 2026-02-15 | Lifecycle guards have ZERO side effects | Guards validate transitions only — no event emission, no HTTP calls, no ledger posts. Side effects happen in the calling lifecycle function after guard approval. Prevents partial state on guard rejection. | Platform Orchestrator + ChatGPT |
| 2026-02-17 | Credit notes are append-only compensating entries | Matches double-entry accounting principles. Never modify the original invoice — issue a compensating credit note instead. Makes GL integration reliable and audit trail complete. | Platform Orchestrator |
| 2026-02-17 | Write-offs are one-per-invoice with REVERSAL mutation class | Simplifies v1 — partial write-offs not supported. UNIQUE(invoice_id) enforces constraint at DB level. REVERSAL mutation class signals to GL that this is a compensating entry, not new revenue. | Platform Orchestrator |
| 2026-02-17 | AR aging is a stored projection, not computed at query time | Computing aging buckets on every read is expensive (joins across invoices, charges, credit notes, write-offs, allocations). Stored projection is refreshed on demand and emits an event. | Platform Orchestrator |
| 2026-02-17 | Dunning state machine with optimistic locking (version field) | Prevents concurrent dunning workers from making conflicting state transitions. Monotonic version field allows compare-and-swap without SELECT FOR UPDATE on every read. | Platform Orchestrator + ChatGPT |
| 2026-02-17 | FIFO payment allocation as default (and only v1) strategy | Oldest-due-first is the most common and legally correct allocation strategy. Explicit allocation rows provide audit trail. Single strategy avoids configuration complexity. | Platform Orchestrator |
| 2026-02-17 | Reconciliation matches are immutable | Once a recon run produces match decisions, they are never changed. Wrong matches are corrected by running a new reconciliation. Preserves full decision history. | Platform Orchestrator |
| 2026-02-17 | Dunning scheduler uses FOR UPDATE SKIP LOCKED | Enables multiple concurrent dunning workers without double-processing. Claimed rows are invisible to other workers. Atomic: claim + state update + outbox event in one transaction. | Platform Orchestrator + ChatGPT |
| 2026-02-17 | Usage billing uses billed_at sentinel + FOR UPDATE SKIP LOCKED | Each usage record can be billed at most once. `billed_at` column marks billing completion. `FOR UPDATE SKIP LOCKED` prevents concurrent bill runs from selecting the same rows. | Platform Orchestrator |
| 2026-02-17 | Tax quote cache with request_hash determinism | Same inputs always produce the same tax — provider is called at most once per unique input combination. Prevents tax amount changes between quote and invoice finalization. | Platform Orchestrator + ChatGPT |
| 2026-02-19 | Party Master validation is optional (party_id nullable) | Not all tenants use Party Master. AR must function without it. When party_id is provided, validation ensures the party exists and belongs to the same tenant. | Platform Orchestrator |
| 2026-02-22 | Invoice lifecycle events (ar.invoice_opened, ar.invoice_paid) with deterministic UUID v5 | Phase 51 cash flow forecasting requires real-time invoice lifecycle events. Idempotency key pattern: `ar.events.<event_type>:<invoice_id>` → UUID v5. ON CONFLICT DO NOTHING ensures replay safety. | Platform Orchestrator |
| 2026-02-22 | Payment succeeded consumer uses event_id deduplication + status guard | Double protection: (1) `processed_events` table prevents re-processing the same event, (2) `UPDATE WHERE status != 'paid'` prevents re-transitioning already-paid invoices. Belt and suspenders. | Platform Orchestrator |

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`
