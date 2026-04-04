# Module Authority Matrix — 7D Solutions Platform

## Purpose
This document defines **domain authority**, **state ownership**, and **allowed mutations** per module.
It is the single source of truth for: "Who owns what?" and "Who is allowed to change what?"

**Non-negotiable rules**
- Modules communicate only via **OpenAPI contracts** and **event contracts**.
- No cross-module DB access (read or write).
- No cross-module source imports.
- Modules are **plug-and-play** and **independently versioned**.
- **Option A locked:** AR drives payment collection via event command; Payments executes; AR applies results.
- **GL posting is event-driven only:** modules emit `gl.posting.requested`; only GL writes journal entries.

---

## Legend
- **Owns**: definitive source of truth (DB + invariants + lifecycle)
- **May mutate**: allowed to change this state (by definition of ownership)
- **Produces**: events this module emits as facts/commands
- **Consumes**: events this module ingests (must be idempotent)

---

## Authority Table (Current + Planned)

### Platform (Tier 1)

| Module | Owns | May mutate | Produces | Consumes |
|---|---|---|---|---|
| `platform/identity-auth` | tenants, users, roles, permissions, auth sessions | yes | `auth.*` (as defined by auth contracts) | none |

### Financial (Tier 2)

| Module | Owns | May mutate | Produces | Consumes |
|---|---|---|---|---|
| `modules/ar` | customers (billing context), invoices, invoice lines, AR ledger state, allocations/payment applications, credits/adjustments, AR disputes, AR reporting views | yes | `ar.invoice_opened`, `ar.invoice_paid`, `ar.payment.collection.requested` (command), `ar.payment_allocated`, `ar.credit_note_issued`, `ar.invoice_written_off`, `ar.usage_captured`, `ar.usage_invoiced`, `ar.recon_run_started`, `ar.recon_match_applied`, `ar.dunning_state_changed`, `ar.invoice_suspended`, `ar.invoice_settled_fx`, `tax.*`, `gl.posting.requested` | `payments.payment.*`, `payments.refund.*`, `payments.dispute.*`, `gl.posting.accepted`, `gl.posting.rejected` |
| `modules/gl` | chart of accounts, journal entries, journal entry lines, account balances, periods, accruals, FX rates, FX revaluations, revenue recognition contracts/schedules | yes | `gl.posting.accepted`, `gl.posting.rejected`, `gl.accrual_created`, `gl.accrual_reversed`, `gl.fx_revaluation_posted`, `gl.fx_realized_posted`, `fx.rate_updated`, `revrec.contract_created`, `revrec.schedule_created`, `revrec.recognition_posted`, `revrec.contract_modified` | `gl.posting.requested` |
| `modules/ap` | vendors, purchase orders, PO lines, PO receipt links, vendor bills, three-way match, AP allocations, payment runs, payment run items/executions, AP tax snapshots | yes | `ap.po_created`, `ap.po_approved`, `ap.po_closed`, `ap.po_line_received_linked`, `ap.vendor_created`, `ap.vendor_updated`, `ap.vendor_bill_created`, `ap.vendor_bill_matched`, `ap.vendor_bill_approved`, `ap.vendor_bill_voided`, `ap.payment_run_created`, `ap.payment_executed` | `inventory.item_received` |
| `modules/payments` | processor integrations, payment intents, payment captures, refunds execution state, webhook ingestion + verification, customer/payment method references (no secrets) | yes | `payments.payment.succeeded`, `payments.payment.failed`, `payments.refund.succeeded`, `payments.refund.failed`, `payments.dispute.*` | `ar.payment.collection.requested` (command) |
| `modules/subscriptions` | subscriptions/service agreements, schedules, proration policy flags, bill-run state, plan templates | yes | `subscriptions.*` (facts) + **OpenAPI command to AR** to create/issue invoice | `ar.invoice_suspended` |
| `modules/treasury` | bank accounts, bank transactions, statement imports, reconciliation state, cash position | yes | none (internal state only) | `payments.payment.succeeded`, `ap.payment_executed` |
| `modules/fixed-assets` | asset categories, assets, depreciation schedules, depreciation runs, disposals, AP capitalizations | yes | `fa_category.category_created`, `fa_asset.asset_created`, `fa_asset.asset_updated`, `fa_asset.asset_deactivated`, `fa_depreciation_run.depreciation_run_completed`, `fa_disposal.asset_disposed`, `gl.posting.requested` | `ap.vendor_bill_approved` |
| `modules/consolidation` | consolidation configs, consolidation caches, elimination postings | yes | none (internal aggregation only) | reads GL/AR/AP data via HTTP APIs |

### Operations (Tier 2)

| Module | Owns | May mutate | Produces | Consumes |
|---|---|---|---|---|
| `modules/inventory` | items, inventory ledger, FIFO layers, reservations, on-hand projections, UOMs, lots, serial instances, status buckets, locations, status transfers, adjustments, cycle counts, transfers, reorder policies, valuation snapshots, low stock state | yes | `inventory.item_received`, `inventory.item_issued`, `inventory.adjusted`, `inventory.transfer_completed`, `inventory.low_stock_triggered`, `inventory.cycle_count_submitted`, `inventory.cycle_count_approved`, `inventory.status_changed`, `inventory.valuation_snapshot_created` | none |
| `modules/shipping-receiving` | shipments, shipment_lines, sr_events_outbox, sr_processed_events (shipment_status_history planned) | inventory stock ledger via Inventory API (receipts/issues) at lifecycle boundaries | `shipping_receiving.shipment_created`, `shipping_receiving.shipment_status_changed`, `shipping_receiving.inbound_closed`, `shipping_receiving.outbound_shipped`, `shipping_receiving.outbound_delivered` | `ap.po.approved`, `sales.so.released` |
| `modules/party` | parties (orgs, people), party external refs, contacts, addresses | yes | `party.created`, `party.updated`, `party.deactivated` | none |
| `modules/maintenance` | work orders, preventive maintenance plans, meter readings, parts/labor records, tenant config | yes | `maintenance.work_order.created`, `maintenance.work_order.status_changed`, `maintenance.work_order.completed`, `maintenance.work_order.closed`, `maintenance.work_order.cancelled`, `maintenance.work_order.overdue`, `maintenance.meter_reading.recorded`, `maintenance.plan.due`, `maintenance.plan.assigned` | none |
| `modules/timekeeping` | employees, projects, timesheet entries, approvals, allocations, exports, billing rates | yes | `timesheet_entry.created`, `timesheet_entry.corrected`, `timesheet_entry.voided`, `timesheet.submitted`, `timesheet.approved`, `timesheet.rejected`, `timesheet.recalled`, `export_run.completed`, `timekeeping.labor_cost`, `timekeeping.billable_time` | none |

### Cross-Cutting (Tier 2)

| Module | Owns | May mutate | Produces | Consumes |
|---|---|---|---|---|
| `modules/integrations` | connector configs, external refs, webhook endpoints, webhook ingest logs | yes | `connector.registered`, `external_ref.created`, `external_ref.updated`, `external_ref.deleted`, `webhook.received`, `webhook.routed` | various (webhook routing) |
| `modules/notifications` | scheduled notifications, delivery status, dead-letter queue, event processing state | yes | `notifications.delivery.succeeded`, `notifications.low_stock.alert.created`, `notifications.close_calendar.reminder` | `ar.invoice_opened`, `payments.payment.succeeded`, `payments.payment.failed`, `inventory.low_stock_triggered` (handler built, not yet wired) |
| `modules/reporting` | report definitions, reporting caches, forecast caches | no (read-only aggregation) | none | `gl.posting.requested`, `payments.payment.succeeded`, `ap.vendor_bill_created`, `ap.vendor_bill_voided`, `ap.payment_executed`, `inventory.valuation_snapshot_created`, `ar.invoice_opened`, `ar.invoice_paid`, `ar.ar_aging_updated` |
| `modules/pdf-editor` | form templates, form fields, form submissions | yes | `pdf.form.submitted`, `pdf.document.generated` (planned) | none |

### Product (Tier 3)

| Module | Owns | May mutate | Produces | Consumes |
|---|---|---|---|---|
| `modules/ttp` | TTP tenants, service configs, metering events, billing runs, billing run items, billing traces | yes | `ttp.billing_run.created`, `ttp.billing_run.completed`, `ttp.billing_run.failed`, `ttp.party.invoiced` | none |

---

## Hard Boundary Rules

### AR (Accounts Receivable)
AR is the **financial authority** for invoices and receivables:
- Only AR may change invoice state (draft/issued/paid/etc.)
- Payments may never "mark invoice paid" directly
- AR stores **payment method references only** (opaque ids), no secrets/PCI

### Subscriptions
Subscriptions owns scheduling only:
- Subscriptions never stores invoice truth
- Subscriptions creates invoices by calling AR OpenAPI (contract-driven)

### Payments
Payments owns processor truth only:
- Payments never mutates AR state
- Payments emits results (`payments.payment.*`) and AR applies them
- Payments owns webhook verification and idempotency for processor events

### Notifications
Notifications is delivery only:
- No financial decisions
- No coupling to internal DB of other modules
- Reacts to facts and sends messages

---

## Required Invariants (Boundary-Level)
1. No module writes to another module's tables.
2. No module imports another module's source code.
3. All cross-module coordination is by **contract** (OpenAPI/events).
4. `gl.posting.requested` is the only way to request GL changes.
5. Payment secrets never enter AR.
6. Every event consumer is idempotent (event_id uniqueness).
7. Tenant isolation is mandatory: every record and event carries tenant_id.

---

## Notes on "Billing"
"Billing" is a **composed capability**, not a module:
- TrashTech billing = Subscriptions + AR + Payments + Notifications (composed at product layer)
