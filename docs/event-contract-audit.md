# Event Contract Audit

Audit date: 2026-04-01
Bead: bd-155vo

## Methodology

For each of the 16 modules with `[events.publish]` in `module.toml`, the audit traces:

1. The **outbox table** where events are stored
2. The **publisher** that reads from the outbox and publishes to NATS
3. The **subject format** the publisher applies (some add a prefix, some publish the event_type directly)
4. The **resolved NATS subject** that actually goes on the wire

For each `.consumer()` call in any `main.rs`, the audit checks whether a publisher
produces a subject that matches the subscription string.

---

## Publisher Inventory

### Modules with Custom Publishers

| Module | Outbox Table | Subject Format | Double-Prefix? |
|--------|-------------|----------------|----------------|
| AP | `events_outbox` | `ap.events.{event_type}` | YES — event_type already starts with `ap.` |
| AR | `events_outbox` | `ar.events.{event_type}` (or `gl.events.{stripped}` for `gl.*`) | YES — event_type already starts with `ar.` |
| Payments | `payments_events_outbox` | `payments.events.{event_type}` | No — event_type is `payment.succeeded` etc. |
| Subscriptions | `events_outbox` | `subscriptions.events.{subject}` | Partial — `subscriptions.status.changed` becomes `subscriptions.events.subscriptions.status.changed` |
| Inventory | `inv_outbox` | `inventory.events.{event_type}` | YES — event_type starts with `inventory.` |
| Notifications | `events_outbox` | `{subject}` (direct, from subject column) | No |
| Maintenance | `events_outbox` | `{event_type}` (direct) | No |
| Shipping-Receiving | `sr_events_outbox` | `{event_type}` (direct) | No |
| Fixed-Assets | `fa_events_outbox` | `{aggregate_type}.{event_type}` | No, but unique format |
| Treasury | `events_outbox` | `treasury.events.{event_type}` | Unknown (needs event_type check) |
| Numbering | `events_outbox` | `{event_type}` (direct) | No |
| Workflow | `events_outbox` | `{event_type}` (direct) | No |
| Integrations | `events_outbox` | `{event_type}` (direct) | No |

### Modules Using SDK Publisher

| Module | Outbox Table | Subject Format |
|--------|-------------|----------------|
| GL | `events_outbox` | `{event_type}` (direct) |
| Production | `production_outbox` | `{event_type}` (direct) |
| PDF-Editor | `events_outbox` | `{subject}` (from subject column) |

---

## Consumer Subscription Cross-Reference

### MISMATCHES

| # | Consumer Module | Subscribes To | Expected Publisher | Actual Published Subject | Status |
|---|----------------|---------------|-------------------|-------------------------|--------|
| 1 | notifications | `ar.events.ar.invoice_opened` | AR | `ar.events.ar.invoice_opened` | **FIXED** (bd-thx8s) |
| 2 | shipping-receiving | `ap.events.ap.po_approved` | AP | `ap.events.ap.po_approved` | **FIXED** (bd-thx8s) |
| 3 | shipping-receiving | `sales.so.released` | (none) | **No sales module exists** | **FIXED** (bd-thx8s) — consumer removed |

#### Mismatch 1: notifications → `ar.events.invoice.issued`

- **Consumer**: `modules/notifications/src/main.rs:27`
- **Problem**: AR has no event type `invoice.issued`. The closest AR event type is `ar.invoice_opened` (constant `EVENT_TYPE_INVOICE_OPENED`), which the AR publisher formats as `ar.events.ar.invoice_opened`.
- **Root cause**: Consumer subscription was written against an assumed subject that was never implemented. The dot-separated `invoice.issued` format doesn't match the underscore-based `ar.invoice_opened` convention either.
- **Impact**: Notifications will never fire for invoice creation/issuance events from AR.

#### Mismatch 2: shipping-receiving → `ap.po.approved`

- **Consumer**: `modules/shipping-receiving/src/main.rs:131`
- **Problem**: AP stores `ap.po_approved` in the outbox. The AP publisher adds `ap.events.` prefix, producing `ap.events.ap.po_approved`. The consumer subscribes to `ap.po.approved` which doesn't match.
- **Root cause**: Three issues compound: (a) AP's double-prefix produces `ap.events.ap.po_approved`; (b) consumer uses dots (`po.approved`) instead of underscores (`po_approved`); (c) consumer omits the `events` segment entirely.
- **Impact**: Shipping-receiving will never auto-create inbound shipments from approved POs.

#### Mismatch 3: shipping-receiving → `sales.so.released`

- **Consumer**: `modules/shipping-receiving/src/main.rs:132`
- **Problem**: There is no `sales` module in the platform. No module publishes to any `sales.*` subject.
- **Root cause**: Consumer was coded against a planned module that doesn't exist yet.
- **Impact**: Shipping-receiving will never auto-create outbound shipments from released sales orders.

### MATCHES (with convention violations)

| # | Consumer Module | Subscribes To | Publisher Module | Published Subject | Status |
|---|----------------|---------------|-----------------|-------------------|--------|
| 4 | fixed-assets | `ap.events.ap.vendor_bill_approved` | AP | `ap.events.ap.vendor_bill_approved` | MATCH (double-prefix) |
| 5 | subscriptions | `ar.events.ar.invoice_suspended` | AR | `ar.events.ar.invoice_suspended` | MATCH (double-prefix) |

These work today because the consumers were coded to expect the double-prefix. But they violate the `nats_subject()` convention of `{module}.events.{entity}.{action}`. If the double-prefix is ever fixed, these consumers will break.

### CLEAN MATCHES

| # | Consumer Module | Subscribes To | Publisher Module | Published Subject | Status |
|---|----------------|---------------|-----------------|-------------------|--------|
| 6 | notifications | `payments.events.payment.succeeded` | Payments | `payments.events.payment.succeeded` | MATCH |
| 7 | notifications | `payments.events.payment.failed` | Payments | `payments.events.payment.failed` | MATCH |
| 8 | ar | `payments.events.payment.succeeded` | Payments | `payments.events.payment.succeeded` | MATCH |
| 9 | payments | `ar.events.payment.collection.requested` | AR | `ar.events.payment.collection.requested` | MATCH |
| 10 | maintenance | `production.workcenter_created` | Production | `production.workcenter_created` | MATCH |
| 11 | maintenance | `production.workcenter_updated` | Production | `production.workcenter_updated` | MATCH |
| 12 | maintenance | `production.workcenter_deactivated` | Production | `production.workcenter_deactivated` | MATCH |
| 13 | maintenance | `production.downtime.started` | Production | `production.downtime.started` | MATCH |
| 14 | maintenance | `production.downtime.ended` | Production | `production.downtime.ended` | MATCH |

---

## Published Event Types by Module

### AP (13 events, all double-prefixed)

| Event Type in Outbox | Resolved NATS Subject | Source File |
|---------------------|-----------------------|-------------|
| `ap.po_created` | `ap.events.ap.po_created` | `events/po.rs:16` |
| `ap.po_approved` | `ap.events.ap.po_approved` | `events/po.rs:19` |
| `ap.po_closed` | `ap.events.ap.po_closed` | `events/po.rs:22` |
| `ap.po_line_received_linked` | `ap.events.ap.po_line_received_linked` | `events/po.rs:25` |
| `ap.vendor_created` | `ap.events.ap.vendor_created` | `events/vendor.rs:15` |
| `ap.vendor_updated` | `ap.events.ap.vendor_updated` | `events/vendor.rs:18` |
| `ap.vendor_bill_created` | `ap.events.ap.vendor_bill_created` | `events/bill.rs:18` |
| `ap.vendor_bill_matched` | `ap.events.ap.vendor_bill_matched` | `events/bill.rs:21` |
| `ap.vendor_bill_approved` | `ap.events.ap.vendor_bill_approved` | `events/bill.rs:24` |
| `ap.vendor_bill_voided` | `ap.events.ap.vendor_bill_voided` | `events/bill.rs:27` |
| `ap.payment_run_created` | `ap.events.ap.payment_run_created` | `events/payment.rs:16` |
| `ap.payment_executed` | `ap.events.ap.payment_executed` | `events/payment.rs:19` |
| `ap.payment_terms_created` | `ap.events.ap.payment_terms_created` | `events/payment_terms.rs` |

### AR (21 events, most double-prefixed)

| Event Type in Outbox | Resolved NATS Subject | Source File |
|---------------------|-----------------------|-------------|
| `ar.invoice_opened` | `ar.events.ar.invoice_opened` | `events/contracts/invoice_lifecycle.rs:21` |
| `ar.invoice_paid` | `ar.events.ar.invoice_paid` | `events/contracts/invoice_lifecycle.rs:24` |
| `ar.invoice_suspended` | `ar.events.ar.invoice_suspended` | `events/contracts/aging_dunning.rs:22` |
| `ar.invoice_written_off` | `ar.events.ar.invoice_written_off` | `events/contracts/credit_writeoff.rs:23` |
| `ar.invoice_settled_fx` | `ar.events.ar.invoice_settled_fx` | `events/contracts/tax_fx.rs:25` |
| `ar.credit_note_issued` | `ar.events.ar.credit_note_issued` | `events/contracts/credit_writeoff.rs:16` |
| `ar.credit_memo_created` | `ar.events.ar.credit_memo_created` | `events/contracts/credit_writeoff.rs:18` |
| `ar.credit_memo_approved` | `ar.events.ar.credit_memo_approved` | `events/contracts/credit_writeoff.rs:20` |
| `ar.dunning_state_changed` | `ar.events.ar.dunning_state_changed` | `events/contracts/aging_dunning.rs:19` |
| `ar.ar_aging_updated` | `ar.events.ar.ar_aging_updated` | `events/contracts/aging_dunning.rs:16` |
| `ar.usage_captured` | `ar.events.ar.usage_captured` | `events/contracts/usage.rs:15` |
| `ar.usage_invoiced` | `ar.events.ar.usage_invoiced` | `events/contracts/usage.rs:18` |
| `ar.payment_allocated` | `ar.events.ar.payment_allocated` | `events/contracts/recon_allocation.rs:26` |
| `ar.recon_run_started` | `ar.events.ar.recon_run_started` | `events/contracts/recon_allocation.rs:17` |
| `ar.recon_match_applied` | `ar.events.ar.recon_match_applied` | `events/contracts/recon_allocation.rs:20` |
| `ar.recon_exception_raised` | `ar.events.ar.recon_exception_raised` | `events/contracts/recon_allocation.rs:23` |
| `ar.milestone_invoice_created` | `ar.events.ar.milestone_invoice_created` | `events/contracts/progress_billing.rs:16` |
| `payment.collection.requested` | `ar.events.payment.collection.requested` | `http/invoices.rs:568` (inline) |
| `gl.posting.requested` | `gl.events.posting.requested` | `http/invoices.rs:623` (inline, routed to GL namespace) |
| `tax.committed` | `ar.events.tax.committed` | `events/contracts/tax_fx.rs:16` |
| `tax.voided` | `ar.events.tax.voided` | `events/contracts/tax_fx.rs:19` |

### Payments (2 events)

| Event Type in Outbox | Resolved NATS Subject | Source File |
|---------------------|-----------------------|-------------|
| `payment.succeeded` | `payments.events.payment.succeeded` | `handlers.rs:57`, `lifecycle.rs:275` |
| `payment.failed` | `payments.events.payment.failed` | `handlers.rs:95` |

### Production (19 events, published directly)

| Event Type in Outbox | Resolved NATS Subject | Source File |
|---------------------|-----------------------|-------------|
| `production.work_order_created` | `production.work_order_created` | `events/mod.rs:33` |
| `production.work_order_released` | `production.work_order_released` | `events/mod.rs:34` |
| `production.work_order_closed` | `production.work_order_closed` | `events/mod.rs:35` |
| `production.component_issue.requested` | `production.component_issue.requested` | `events/mod.rs:36` |
| `production.component_issued` | `production.component_issued` | `events/mod.rs:37` |
| `production.operation_started` | `production.operation_started` | `events/mod.rs:38` |
| `production.operation_completed` | `production.operation_completed` | `events/mod.rs:39` |
| `production.fg_received` | `production.fg_received` | `events/mod.rs:40` |
| `production.fg_receipt.requested` | `production.fg_receipt.requested` | `events/mod.rs:41` |
| `production.workcenter_created` | `production.workcenter_created` | `events/mod.rs:42` |
| `production.workcenter_updated` | `production.workcenter_updated` | `events/mod.rs:43` |
| `production.workcenter_deactivated` | `production.workcenter_deactivated` | `events/mod.rs:44` |
| `production.routing_created` | `production.routing_created` | `events/mod.rs:45` |
| `production.routing_updated` | `production.routing_updated` | `events/mod.rs:46` |
| `production.routing_released` | `production.routing_released` | `events/mod.rs:47` |
| `production.time_entry_created` | `production.time_entry_created` | `events/mod.rs:48` |
| `production.time_entry_stopped` | `production.time_entry_stopped` | `events/mod.rs:49` |
| `production.downtime.started` | `production.downtime.started` | `events/mod.rs:50` |
| `production.downtime.ended` | `production.downtime.ended` | `events/mod.rs:51` |

### Subscriptions (2 events)

| Event Type in Outbox | Resolved NATS Subject | Source File |
|---------------------|-----------------------|-------------|
| `billrun.completed` | `subscriptions.events.billrun.completed` | `http/bill_run.rs:286` |
| `subscriptions.status.changed` | `subscriptions.events.subscriptions.status.changed` | `lifecycle/transitions.rs:56,108` |

### Inventory (20 events, all double-prefixed)

| Event Type in Outbox | Resolved NATS Subject | Source File |
|---------------------|-----------------------|-------------|
| `inventory.item_received` | `inventory.events.inventory.item_received` | `events/contracts.rs:31` |
| `inventory.item_issued` | `inventory.events.inventory.item_issued` | `events/contracts.rs:35` |
| `inventory.adjusted` | `inventory.events.inventory.adjusted` | `events/contracts.rs:38` |
| `inventory.transfer_completed` | `inventory.events.inventory.transfer_completed` | `events/contracts.rs:41` |
| `inventory.valuation_snapshot_created` | `inventory.events.inventory.valuation_snapshot_created` | `events/valuation_snapshot_created.rs:23` |
| `inventory.valuation_run_completed` | `inventory.events.inventory.valuation_run_completed` | `events/valuation_run_completed.rs:22` |
| `inventory.status_changed` | `inventory.events.inventory.status_changed` | `events/status_changed.rs:24` |
| `inventory.low_stock_triggered` | `inventory.events.inventory.low_stock_triggered` | `events/low_stock_triggered.rs:15` |
| `inventory.item_change_recorded` | `inventory.events.inventory.item_change_recorded` | `events/item_change_recorded.rs:16` |
| `inventory.classification_assigned.v1` | `inventory.events.inventory.classification_assigned.v1` | `events/classification_assigned.rs:14` |
| `inventory.label_generated.v1` | `inventory.events.inventory.label_generated.v1` | `events/label_generated.rs:15` |
| `inventory.lot_merged.v1` | `inventory.events.inventory.lot_merged.v1` | `events/lot_merged.rs:15` |
| `inventory.lot_split.v1` | `inventory.events.inventory.lot_split.v1` | `events/lot_split.rs:15` |
| `inventory.expiry_alert.v1` | `inventory.events.inventory.expiry_alert.v1` | `events/expiry_alert.rs:10` |
| `inventory.expiry_set.v1` | `inventory.events.inventory.expiry_set.v1` | `events/expiry_set.rs:10` |
| `inventory.make_buy_changed` | `inventory.events.inventory.make_buy_changed` | `events/make_buy_changed.rs:14` |
| `inventory.item_revision_created` | `inventory.events.inventory.item_revision_created` | `events/revision_created.rs:14` |
| `inventory.item_revision_activated` | `inventory.events.inventory.item_revision_activated` | `events/revision_activated.rs:15` |
| `inventory.item_revision_policy_updated` | `inventory.events.inventory.item_revision_policy_updated` | `events/revision_policy_updated.rs:14` |
| `inventory.cycle_count_submitted` | `inventory.events.inventory.cycle_count_submitted` | `events/cycle_count_submitted.rs:24` |
| `inventory.cycle_count_approved` | `inventory.events.inventory.cycle_count_approved` | `events/cycle_count_approved.rs:24` |

### Maintenance (17 events, published directly)

| Event Type in Outbox | Resolved NATS Subject | Source File |
|---------------------|-----------------------|-------------|
| `maintenance.work_order.created` | `maintenance.work_order.created` | `events/subjects.rs:8` |
| `maintenance.work_order.status_changed` | `maintenance.work_order.status_changed` | `events/subjects.rs:9` |
| `maintenance.work_order.completed` | `maintenance.work_order.completed` | `events/subjects.rs:10` |
| `maintenance.work_order.closed` | `maintenance.work_order.closed` | `events/subjects.rs:11` |
| `maintenance.work_order.cancelled` | `maintenance.work_order.cancelled` | `events/subjects.rs:12` |
| `maintenance.work_order.overdue` | `maintenance.work_order.overdue` | `events/subjects.rs:13` |
| `maintenance.meter_reading.recorded` | `maintenance.meter_reading.recorded` | `events/subjects.rs:16` |
| `maintenance.plan.due` | `maintenance.plan.due` | `events/subjects.rs:19` |
| `maintenance.plan.assigned` | `maintenance.plan.assigned` | `events/subjects.rs:20` |
| `maintenance.asset.created` | `maintenance.asset.created` | `events/subjects.rs:23` |
| `maintenance.asset.updated` | `maintenance.asset.updated` | `events/subjects.rs:24` |
| `maintenance.downtime.recorded` | `maintenance.downtime.recorded` | `events/subjects.rs:27` |
| `maintenance.calibration.created` | `maintenance.calibration.created` | `events/subjects.rs:30` |
| `maintenance.calibration.completed` | `maintenance.calibration.completed` | `events/subjects.rs:31` |
| `maintenance.calibration.event_recorded` | `maintenance.calibration.event_recorded` | `events/subjects.rs:32` |
| `maintenance.calibration.status_changed` | `maintenance.calibration.status_changed` | `events/subjects.rs:33` |
| `maintenance.asset.out_of_service_changed` | `maintenance.asset.out_of_service_changed` | `events/subjects.rs:36` |

### Shipping-Receiving (12 events, published directly)

| Event Type in Outbox | Resolved NATS Subject | Source File |
|---------------------|-----------------------|-------------|
| `shipping_receiving.shipment_created` | `shipping_receiving.shipment_created` | `events/contracts.rs:32` |
| `shipping_receiving.shipment_status_changed` | `shipping_receiving.shipment_status_changed` | `events/contracts.rs:35` |
| `shipping_receiving.inbound_closed` | `shipping_receiving.inbound_closed` | `events/contracts.rs:38` |
| `shipping_receiving.outbound_shipped` | `shipping_receiving.outbound_shipped` | `events/contracts.rs:41` |
| `shipping_receiving.outbound_delivered` | `shipping_receiving.outbound_delivered` | `events/contracts.rs:44` |
| `sr.receipt_routed_to_inspection.v1` | `sr.receipt_routed_to_inspection.v1` | `events/contracts.rs:47` |
| `sr.receipt_routed_to_stock.v1` | `sr.receipt_routed_to_stock.v1` | `events/contracts.rs:50` |
| `sr.rma.received` | `sr.rma.received` | `domain/rma/service.rs:92` |
| `sr.rma.disposition_changed` | `sr.rma.disposition_changed` | `domain/rma/service.rs:93` |
| `sr.carrier_request.created` | `sr.carrier_request.created` | `domain/carrier_requests/service.rs:75` |
| `sr.carrier_request.status_changed` | `sr.carrier_request.status_changed` | `domain/carrier_requests/service.rs:76` |
| `sr.shipping_doc.requested` | `sr.shipping_doc.requested` | `domain/shipping_docs/service.rs:70` |
| `sr.shipping_doc.status_changed` | `sr.shipping_doc.status_changed` | `domain/shipping_docs/service.rs:71` |

### GL (11 events, published directly via SDK)

| Event Type in Outbox | Resolved NATS Subject | Source File |
|---------------------|-----------------------|-------------|
| `fx.rate_updated` | `fx.rate_updated` | `events/contracts/fx.rs:21` |
| `gl.fx_revaluation_posted` | `gl.fx_revaluation_posted` | `events/contracts/fx.rs:24` |
| `gl.fx_realized_posted` | `gl.fx_realized_posted` | `events/contracts/fx.rs:27` |
| `gl.export.requested` | `gl.export.requested` | `exports/service.rs:108` |
| `gl.export.completed` | `gl.export.completed` | `exports/service.rs:109` |
| `gl.accrual_created` | `gl.accrual_created` | `events/contracts/accruals.rs:23` |
| `gl.accrual_reversed` | `gl.accrual_reversed` | `events/contracts/accruals.rs:26` |
| `revrec.contract_created` | `revrec.contract_created` | `revrec/contracts/mod.rs:61` |
| `revrec.schedule_created` | `revrec.schedule_created` | `revrec/contracts/mod.rs:64` |
| `revrec.recognition_posted` | `revrec.recognition_posted` | `revrec/contracts/mod.rs:67` |
| `revrec.contract_modified` | `revrec.contract_modified` | `revrec/contracts/mod.rs:70` |

### Fixed-Assets (6 events, `{aggregate_type}.{event_type}` format)

| Event Type | Aggregate Type | Resolved NATS Subject | Source File |
|-----------|---------------|----------------------|-------------|
| `category_created` | `fa_category` | `fa_category.category_created` | `domain/assets/service.rs:74` |
| `asset_created` | `fa_asset` | `fa_asset.asset_created` | `domain/assets/service.rs:280` |
| `asset_updated` | `fa_asset` | `fa_asset.asset_updated` | `domain/assets/service.rs:339` |
| `asset_deactivated` | `fa_asset` | `fa_asset.asset_deactivated` | `domain/assets/service.rs:392` |
| `asset_disposed` | `fa_disposal` | `fa_disposal.asset_disposed` | `domain/disposals/service.rs:211` |
| `depreciation_run_completed` | `fa_depreciation_run` | `fa_depreciation_run.depreciation_run_completed` | `domain/depreciation/service.rs:256` |

---

## Systemic Issues

### 1. Double-Prefix Convention Violation (AP, AR, Inventory)

AP, AR, and Inventory store event types that already include the module name as a prefix
(e.g., `ap.po_approved`, `ar.invoice_opened`, `inventory.item_received`). Their publishers
then prepend `{module}.events.`, producing subjects like `ap.events.ap.po_approved`.

The `nats_subject()` function in `platform-contracts` defines the standard as
`nats_subject("ar", "invoice.created")` → `"ar.events.invoice.created"`. By this standard,
the event_type should be `po_approved` (not `ap.po_approved`) so the published subject
becomes `ap.events.po_approved`.

**Affected modules**: AP (13 events), AR (17 of 21 events), Inventory (20+ events)

**Risk**: Any new consumer using the standard convention will fail to match. Existing
consumers that work (fixed-assets, subscriptions) are coded to expect the double-prefix.

### 2. Inconsistent Publisher Patterns

Five different publisher patterns exist across 16 modules:
- **Prefix-adding** (AP, AR, Payments, Subscriptions, Inventory, Treasury): `{module}.events.{event_type}`
- **Direct** (Production, Maintenance, Shipping-Receiving, Numbering, Workflow, Integrations, GL): `{event_type}` as-is
- **Subject-column** (Notifications, PDF-Editor): reads a separate `subject` column
- **Aggregate-based** (Fixed-Assets): `{aggregate_type}.{event_type}`
- **GL-routing** (AR only): routes `gl.*` events to `gl.events.*` namespace

This inconsistency makes it impossible to predict a module's NATS subject from its event_type
without reading the publisher code.

### 3. Orphaned Consumer (sales.so.released)

Shipping-receiving subscribes to `sales.so.released` but no sales module exists.
This consumer was likely written against a planned module that was never built.

---

## Recommendation

The dependent bead `bd-thx8s` (Fix event subject mismatches) should address mismatches
1-3 in priority order:

1. **shipping-receiving → AP PO approved**: fix consumer subject to `ap.events.ap.po_approved`
   (match the current publisher behavior) or fix AP to stop double-prefixing
2. **notifications → AR invoice issued**: either add an `invoice.issued` event to AR,
   or change the consumer to subscribe to `ar.events.ar.invoice_opened`
3. **shipping-receiving → sales.so.released**: remove or gate behind feature flag until
   a sales module exists
