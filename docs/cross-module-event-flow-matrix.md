# Cross-Module Event Flow Matrix

> Generated: 2026-03-31 | Bead: bd-31h8l | Author: PurpleCliff

## Summary

Audited all 25 modules for outbox publishers and event consumers. Found **30 cross-module event flows**. Of these:

- **4 flows are WIRED and MATCHED** (publisher subject matches consumer subscription)
- **11 flows have SUBJECT MISMATCHES** (publisher and consumer use different subjects)
- **10 flows have NO PUBLISHER RUNNING** (outbox entries written but never relayed to bus)
- **1 flow is a DEAD CONSUMER** (code exists but not spawned in main.rs)
- **1 flow has NO PUBLISHER MODULE** (consumer listens for events nobody emits)
- **3 flows are INTERNAL to GL** (GL consumes its own events — posted by AR cross-module)

---

## Event Flow Matrix

### Legend

| Status | Meaning |
|--------|---------|
| LIVE | Publisher subject matches consumer subscription, both wired |
| SUBJECT-MISMATCH | Publisher publishes to different subject than consumer subscribes to |
| NO-PUBLISHER | Outbox entries written but relay task not spawned in main.rs |
| DEAD-CONSUMER | Consumer code exists but not wired in main.rs |
| ORPHAN | Consumer listens for events that no module publishes |

---

### 1. AR -> Payments (LIVE)

| Field | Value |
|-------|-------|
| Publisher | AR (`ar/src/http/invoices.rs:566`) |
| Event type | `payment.collection.requested` |
| Published subject | `ar.events.payment.collection.requested` |
| Consumer | Payments (`payments/src/consumer_task.rs:25`) |
| Subscribed subject | `ar.events.payment.collection.requested` |
| Status | **LIVE** |

### 2. Payments -> AR (LIVE)

| Field | Value |
|-------|-------|
| Publisher | Payments (`payments/src/lifecycle.rs:275`, `handlers.rs:57`) |
| Event type | `payment.succeeded` |
| Published subject | `payments.events.payment.succeeded` |
| Consumer | AR (`ar/src/consumer_tasks.rs:22`) |
| Subscribed subject | `payments.events.payment.succeeded` |
| Status | **LIVE** |

### 3. Payments -> Notifications: payment succeeded (LIVE)

| Field | Value |
|-------|-------|
| Publisher | Payments |
| Event type | `payment.succeeded` |
| Published subject | `payments.events.payment.succeeded` |
| Consumer | Notifications (`notifications/src/consumer_tasks.rs:154`) |
| Subscribed subject | `payments.events.payment.succeeded` |
| Status | **LIVE** |

### 4. Payments -> Notifications: payment failed (LIVE)

| Field | Value |
|-------|-------|
| Publisher | Payments (`payments/src/handlers.rs:95`) |
| Event type | `payment.failed` |
| Published subject | `payments.events.payment.failed` |
| Consumer | Notifications (`notifications/src/consumer_tasks.rs:284`) |
| Subscribed subject | `payments.events.payment.failed` |
| Status | **LIVE** |

---

### 5. AR -> Notifications: invoice issued (SUBJECT-MISMATCH)

| Field | Value |
|-------|-------|
| Publisher | AR (`ar/src/http/invoices.rs:189`) |
| Event type | `ar.invoice_opened` (EVENT_TYPE_INVOICE_OPENED) |
| Published subject | `ar.events.ar.invoice_opened` |
| Consumer | Notifications (`notifications/src/consumer_tasks.rs:24`) |
| Subscribed subject | `ar.events.invoice.issued` |
| Status | **SUBJECT-MISMATCH** |
| Impact | Invoice-issued notifications are never sent |

### 6. AR -> GL: posting requested (LIVE via cross-module routing)

| Field | Value |
|-------|-------|
| Publisher | AR (`ar/src/http/invoices.rs:621`) |
| Event type | `gl.posting.requested` (starts with "gl." so routed to GL namespace) |
| Published subject | `gl.events.posting.requested` |
| Consumer | GL (`gl/src/consumers/gl_posting_consumer.rs:28`) |
| Subscribed subject | `gl.events.posting.requested` |
| Status | **LIVE** (AR publisher has special gl.* prefix routing) |

### 7. AR -> GL: tax committed (SUBJECT-MISMATCH)

| Field | Value |
|-------|-------|
| Publisher | AR (`ar/src/finalization.rs:543`) |
| Event type | `tax.committed` (EVENT_TYPE_TAX_COMMITTED) |
| Published subject | `ar.events.tax.committed` (does NOT start with "gl." so goes to ar.events.*) |
| Consumer | GL (`gl/src/consumers/ar_tax_liability.rs:31`) |
| Subscribed subject | `tax.committed` |
| Status | **SUBJECT-MISMATCH** |
| Impact | Tax liability GL entries never posted |

### 8. AR -> GL: tax voided (SUBJECT-MISMATCH)

| Field | Value |
|-------|-------|
| Publisher | AR (`ar/src/finalization.rs:677`) |
| Event type | `tax.voided` (EVENT_TYPE_TAX_VOIDED) |
| Published subject | `ar.events.tax.voided` |
| Consumer | GL (`gl/src/consumers/ar_tax_liability.rs:103`) |
| Subscribed subject | `tax.voided` |
| Status | **SUBJECT-MISMATCH** |
| Impact | Tax voiding GL entries never reversed |

### 9. AR -> GL: credit note issued (SUBJECT-MISMATCH)

| Field | Value |
|-------|-------|
| Publisher | AR (`ar/src/credit_notes.rs`) |
| Event type | `ar.credit_note_issued` |
| Published subject | `ar.events.ar.credit_note_issued` |
| Consumer | GL (`gl/src/consumers/gl_credit_note_consumer.rs:145`) |
| Subscribed subject | `ar.credit_note_issued` |
| Status | **SUBJECT-MISMATCH** |
| Impact | Credit note GL entries never posted |

### 10. AR -> GL: invoice written off (SUBJECT-MISMATCH)

| Field | Value |
|-------|-------|
| Publisher | AR (`ar/src/write_offs.rs`) |
| Event type | `ar.invoice_written_off` |
| Published subject | `ar.events.ar.invoice_written_off` |
| Consumer | GL (`gl/src/consumers/gl_writeoff_consumer.rs:150`) |
| Subscribed subject | `ar.invoice_written_off` |
| Status | **SUBJECT-MISMATCH** |
| Impact | Write-off GL entries never posted |

### 11. AR -> GL: FX settlement (SUBJECT-MISMATCH)

| Field | Value |
|-------|-------|
| Publisher | AR |
| Event type | `ar.invoice_settled_fx` |
| Published subject | `ar.events.ar.invoice_settled_fx` |
| Consumer | GL (`gl/src/consumers/gl_fx_realized_consumer.rs:214`) |
| Subscribed subject | `ar.invoice_settled_fx` |
| Status | **SUBJECT-MISMATCH** |
| Impact | FX realized gain/loss GL entries never posted |

### 12. AP -> GL: vendor bill approved (SUBJECT-MISMATCH)

| Field | Value |
|-------|-------|
| Publisher | AP (`ap/src/outbox/mod.rs:125`) |
| Event type | `ap.vendor_bill_approved` |
| Published subject | `ap.events.ap.vendor_bill_approved` |
| Consumer | GL (`gl/src/consumers/ap_vendor_bill_approved_consumer.rs:315`) |
| Subscribed subject | `ap.vendor_bill_approved` |
| Status | **SUBJECT-MISMATCH** |
| Impact | AP bill expense GL entries never posted |

### 13. AP -> Fixed-Assets: vendor bill approved (LIVE)

| Field | Value |
|-------|-------|
| Publisher | AP (`ap/src/outbox/mod.rs:125`) |
| Event type | `ap.vendor_bill_approved` |
| Published subject | `ap.events.ap.vendor_bill_approved` |
| Consumer | Fixed-Assets (`fixed-assets/src/consumers/ap_bill_approved.rs:121`) |
| Subscribed subject | `ap.events.ap.vendor_bill_approved` |
| Status | **LIVE** |

### 14. AP -> Shipping-Receiving: PO approved (SUBJECT-MISMATCH)

| Field | Value |
|-------|-------|
| Publisher | AP (`ap/src/outbox/mod.rs:125`) |
| Event type | `ap.po_approved` |
| Published subject | `ap.events.ap.po_approved` |
| Consumer | Shipping-Receiving (`shipping-receiving/src/consumers/po_approved.rs:22`) |
| Subscribed subject | `ap.po.approved` |
| Status | **SUBJECT-MISMATCH** |
| Impact | PO approval never triggers expected receipt in SR |

### 15. Fixed-Assets -> GL: depreciation run completed (LIVE)

| Field | Value |
|-------|-------|
| Publisher | Fixed-Assets (`fixed-assets/src/outbox/mod.rs:89`) |
| Event type | `depreciation_run_completed`, aggregate: `fa_depreciation_run` |
| Published subject | `fa_depreciation_run.depreciation_run_completed` |
| Consumer | GL (`gl/src/consumers/fixed_assets_depreciation.rs:24`) |
| Subscribed subject | `fa_depreciation_run.depreciation_run_completed` |
| Status | **LIVE** |

### 16. ??? -> GL: reversal requested (ORPHAN — test-only publisher)

| Field | Value |
|-------|-------|
| Publisher | None in production (only in GL tests) |
| Published subject | `gl.events.entry.reverse.requested` |
| Consumer | GL (`gl/src/consumers/gl_reversal_consumer.rs:28`) |
| Subscribed subject | `gl.events.entry.reverse.requested` |
| Status | **ORPHAN** (no production publisher) |
| Impact | GL reversal consumer wired but never receives events |

---

### 17-22. Production -> Multiple consumers (NO-PUBLISHER)

Production writes events to `production_outbox` table but **has no outbox relay task in main.rs**. All downstream consumers are starved.

| # | Event type | Consumer Module | Consumer file | Status |
|---|-----------|----------------|---------------|--------|
| 17 | `production.workcenter_created` | Maintenance | `consumers/production_workcenter_bridge.rs` | NO-PUBLISHER |
| 18 | `production.workcenter_updated` | Maintenance | `consumers/production_workcenter_bridge.rs` | NO-PUBLISHER |
| 19 | `production.workcenter_deactivated` | Maintenance | `consumers/production_workcenter_bridge.rs` | NO-PUBLISHER |
| 20 | `production.downtime.started` | Maintenance | `consumers/production_downtime_bridge.rs` | NO-PUBLISHER |
| 21 | `production.downtime.ended` | Maintenance | `consumers/production_downtime_bridge.rs` | NO-PUBLISHER |
| 22 | `production.component_issue.requested` | Inventory | `consumers/component_issue_consumer.rs` | NO-PUBLISHER |
| 23 | `production.fg_receipt.requested` | Inventory | `consumers/fg_receipt_consumer.rs` | NO-PUBLISHER |
| 24 | `production.operation_completed` | Quality-Inspection | `consumers/production_event_bridge.rs` | NO-PUBLISHER |
| 25 | `production.fg_receipt.requested` | Quality-Inspection | `consumers/production_event_bridge.rs` | NO-PUBLISHER |

### 26-27. Inventory -> GL & QI (NO-PUBLISHER + SUBJECT-MISMATCH)

Inventory has `start_outbox_publisher()` in `event_bus.rs` but it is **not wired in main.rs** (TODO comment on lines 645, 669 references `bd-rbhj1`). Even if wired, subjects wouldn't match.

| # | Event type | Published subject (if wired) | Consumer | Subscribed subject | Status |
|---|-----------|------------------------------|----------|-------------------|--------|
| 26 | `inventory.item_issued` | `inventory.events.inventory.item_issued` | GL | `inventory.item_issued` | NO-PUBLISHER + SUBJECT-MISMATCH |
| 27 | `inventory.item_received` | `inventory.events.inventory.item_received` | GL | `inventory.item_received` | NO-PUBLISHER + SUBJECT-MISMATCH |
| 28 | `inventory.item_received` | `inventory.events.inventory.item_received` | Quality-Inspection | `inventory.item_received` | NO-PUBLISHER + SUBJECT-MISMATCH |

### 29. Timekeeping -> GL: labor cost (NO-PUBLISHER + SUBJECT-MISMATCH)

Timekeeping enqueues events but **has no outbox relay task** and **no event bus wiring** in main.rs.

| Field | Value |
|-------|-------|
| Publisher | Timekeeping (`timekeeping/src/domain/integrations/gl/service.rs:155`) |
| Event type | `timekeeping.labor_cost` |
| Published subject | Unknown — no relay task exists |
| Consumer | GL (`gl/src/consumers/timekeeping_labor_cost.rs:150`) |
| Subscribed subject | `timekeeping.labor_cost` |
| Status | **NO-PUBLISHER** |

### 30. AR -> Subscriptions: invoice suspended (DEAD-CONSUMER)

| Field | Value |
|-------|-------|
| Publisher | AR (`ar/src/dunning/engine.rs:316`) |
| Event type | `ar.invoice_suspended` |
| Published subject | `ar.events.ar.invoice_suspended` |
| Consumer | Subscriptions (`subscriptions/src/consumer.rs:27` — `handle_invoice_suspended`) |
| Wired in main.rs? | **NO** — function exists but never called |
| Status | **DEAD-CONSUMER** |
| Impact | Suspended invoices never trigger subscription suspension |

### 31. ??? -> Shipping-Receiving: SO released (ORPHAN)

| Field | Value |
|-------|-------|
| Publisher | None — no "sales" module exists |
| Consumer | Shipping-Receiving (`shipping-receiving/src/consumers/so_released.rs`) |
| Subscribed subject | `sales.so.released` |
| Status | **ORPHAN** — no publisher module exists |

---

## Modules with Outbox Publisher Tasks Running

| Module | Publisher wired in main.rs? | Subject format |
|--------|-----------------------------|----------------|
| AR | Yes | `ar.events.{event_type}` (special: `gl.*` -> `gl.events.*`) |
| AP | Yes | `ap.events.{event_type}` |
| Payments | Yes (via outbox relay) | `payments.events.{event_type}` |
| Subscriptions | Yes | `subscriptions.events.{event.subject}` |
| Shipping-Receiving | Yes | `{event_type}` (direct) |
| Fixed-Assets | Yes | `{aggregate_type}.{event_type}` |
| Maintenance | Yes | `{event_type}` (direct) |
| Treasury | Yes | Unknown (no cross-module consumers) |
| Workflow | Yes | Unknown (no cross-module consumers) |
| Numbering | Yes | Unknown (no cross-module consumers) |
| Integrations | Yes | `{event_type}` (via relay) |

## Modules MISSING Outbox Publisher Tasks

| Module | Outbox table | Events written | Downstream consumers affected |
|--------|-------------|----------------|-------------------------------|
| **Production** | `production_outbox` | 15 event types | Maintenance (5), Inventory (2), QI (2) |
| **Inventory** | `inv_outbox` | 20+ event types | GL (2), QI (1) |
| **Timekeeping** | events table | 7+ event types | GL (1) |
| GL | None | 0 (pure consumer) | N/A |
| Quality-Inspection | `qi_events_outbox` | 5 event types | None currently |
| Notifications | `events_outbox` | Internal only | N/A |

---

## Root Cause Analysis

### Issue 1: Inconsistent subject naming convention

Three different patterns exist:
1. **Namespaced**: `{module}.events.{event_type}` — AR, AP, Payments, Subscriptions, Inventory
2. **Direct**: `{event_type}` — Shipping-Receiving, Maintenance, Production (events named with module prefix)
3. **Composite**: `{aggregate_type}.{event_type}` — Fixed-Assets

Consumers were written assuming the "direct" pattern (e.g., `tax.committed`), but publishers use the "namespaced" pattern (e.g., `ar.events.tax.committed`).

### Issue 2: Missing outbox relay tasks

Production, Inventory, and Timekeeping all write events to outbox tables but never spawn a background task to relay them to NATS. This is the most impactful gap — 12 downstream consumers receive nothing.

### Issue 3: Dead and orphan consumers

- Subscriptions' `handle_invoice_suspended` exists but is never spawned
- Shipping-Receiving's `so_released` consumer listens for events from a module that doesn't exist

---

## Recommended Fixes (Priority Order)

1. **P0: Wire production outbox relay** — 9 downstream consumers blocked
2. **P0: Wire inventory outbox relay** — 3 downstream consumers blocked (note: also fix subject format to match GL/QI expectations)
3. **P0: Fix GL consumer subjects** — 6 GL consumers subscribe to wrong subjects (need `ar.events.*` or `ap.events.*` prefix)
4. **P1: Wire timekeeping outbox relay** — GL labor cost consumer blocked
5. **P1: Fix SR consumer subjects** — `ap.po.approved` should be `ap.events.ap.po_approved`
6. **P1: Fix notifications invoice consumer subject** — `ar.events.invoice.issued` should be `ar.events.ar.invoice_opened`
7. **P2: Wire subscriptions invoice_suspended consumer** — dead consumer in main.rs
8. **P3: Decide on `sales.so.released`** — either build sales module or remove orphan consumer
