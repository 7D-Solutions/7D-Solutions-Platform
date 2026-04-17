# Outside-Processing Module — Scope, Boundaries, Contracts (draft v0.1)

**7D Solutions Platform**
**Status:** Draft Specification — bd-ixnbs migration
**Date:** 2026-04-16
**Proposed module name:** `outside-processing`
**Source of migration:** Fireproof ERP (`outside_processing/` module, ~1,640 LOC)
**Cross-vertical applicability:** Fireproof (aerospace heat treat / anodize / coat), HuberPower (component machining / overhaul)

---

## 1. Mission & Non-Goals

### Mission
The Outside-Processing module is the **authoritative system for sending goods or materials to external vendors for specialized work and tracking the round-trip**: who it went to, what they were supposed to do, what shipped out, what came back, whether it passed review, and how it ties back to the source work order.

### Non-Goals
Outside-Processing does **NOT**:
- Own purchase orders themselves (delegated to AP — OP creates a service PO in AP and tracks its OP-specific lifecycle on top)
- Own shipping logistics (delegated to Shipping-Receiving — OP emits `shipment.requested.v1`, Shipping-Receiving creates the actual shipment)
- Own vendor records (delegated to AP vendors — OP references `vendor_id`)
- Own quality inspection of returned goods (delegated to Quality-Inspection for formal inspection plans; OP records a lightweight review outcome for operational flow)
- Execute AS9100 certificate-of-conformance review (stays in Fireproof as an overlay — OP records generic `cert_ref` and outcome only)
- Generate corrective actions when returned work fails (CAPA is Fireproof-only per user ruling; verticals wire their own response)

---

## 2. Domain Authority

Outside-Processing is the **source of truth** for:

| Domain Entity | Authority |
|---------------|-----------|
| **OP Orders** | The OP lifecycle: draft → issued → shipped_to_vendor → at_vendor → returned → review_in_progress → closed/cancelled, with its service type, vendor, quantities, dates, work order link |
| **Ship-to-Vendor Events** | Outbound shipment records: date, quantity, UoM, lot/serial, carrier, tracking, packing slip, shipped-by user |
| **Return-from-Vendor Events** | Inbound return records: date, quantity received, condition, lot/serial, carrier, vendor packing slip, re-identification flag |
| **Vendor Review Records** | Lightweight outcome record per return: outcome (accepted/rejected/conditional), reviewer, timestamp, notes, optional opaque `cert_ref` |
| **Re-identification Records** | Identity changes when the material comes back with a new spec/revision/heat-treat-cond after processing |

Outside-Processing is **NOT** authoritative for:
- Vendor master data (AP owns)
- PO financial terms (AP owns the PO)
- Inventory lot state during at-vendor period (Inventory tracks via an "in-transit-out" location; OP just references the lot_id)
- Formal inspection results (Quality-Inspection owns inspection plans/executions)
- Work order / routing / operation definitions (Production owns)

---

## 3. Data Ownership

All tables include `tenant_id`, shared-DB model. Monetary values use integer `*_cents`.

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **op_orders** | OP order header | `id`, `tenant_id`, `op_order_number`, `status` (canonical: draft/issued/shipped_to_vendor/at_vendor/returned/review_in_progress/closed/cancelled), `vendor_id` (ref → AP vendor), `service_type` (free text or tenant-configured code: "heat_treat", "anodize", "machining", "plating", "edm", etc.), `service_description`, `process_spec_ref` (opaque string — aerospace puts AS spec, others put whatever), `part_number` (denormalized; nullable), `part_revision`, `quantity_sent`, `unit_of_measure`, `work_order_id` (ref → Production work order), `operation_id` (nullable — ref → Production operation), `purchase_order_id` (nullable — ref → AP purchase order), `lot_id` (nullable — ref → Inventory lot), `serial_numbers` (array of string, optional), `expected_ship_date`, `expected_return_date`, `estimated_cost_cents`, `actual_cost_cents`, `notes`, `created_by`, `created_at`, `updated_at` |
| **op_ship_events** | Each physical shipment to the vendor (supports partial/split shipments) | `id`, `tenant_id`, `op_order_id`, `ship_date`, `quantity_shipped`, `unit_of_measure`, `lot_number`, `serial_numbers` (array), `carrier_name`, `tracking_number`, `packing_slip_number`, `shipped_by`, `shipping_reference` (FK-like back to Shipping-Receiving shipment), `notes`, `created_at` |
| **op_return_events** | Each physical return from the vendor | `id`, `tenant_id`, `op_order_id`, `received_date`, `quantity_received`, `unit_of_measure`, `condition` (canonical: good/damaged/discrepancy), `discrepancy_notes`, `lot_number`, `serial_numbers` (array), `cert_ref` (opaque string — aerospace puts cert number, others leave null), `vendor_packing_slip`, `carrier_name`, `tracking_number`, `re_identification_required`, `received_by`, `notes`, `created_at` |
| **op_vendor_reviews** | Lightweight review of vendor work | `id`, `tenant_id`, `op_order_id`, `return_event_id`, `outcome` (canonical: accepted/rejected/conditional), `conditions` (nullable — if conditional, what conditions), `rejection_reason` (nullable — if rejected, why), `reviewed_by`, `reviewed_at`, `notes`, `created_at` |
| **op_re_identifications** | Record of material identity change after processing | `id`, `tenant_id`, `op_order_id`, `return_event_id`, `old_part_number`, `old_part_revision`, `new_part_number`, `new_part_revision`, `reason`, `performed_by`, `performed_at`, `created_at` |
| **op_status_labels** | Tenant display labels over canonical statuses | Standard label-table shape: `id`, `tenant_id`, `canonical_status`, `display_label`, `description`, `updated_by`, `updated_at` |
| **op_service_type_labels** | Tenant display labels over `service_type` codes (optional — tenants that configure canonical codes for their industry) | Same shape |

**Note on `service_type`:** unlike `status` which is a fixed canonical enum, `service_type` is intentionally open. Tenants register their own codes (e.g., `heat_treat`, `anodize`, `machining`, `plating`, `edm`) via the labels table, or pass free text. Platform doesn't constrain.

---

## 4. OpenAPI Surface

### 4.1 OP Order Endpoints
- `POST /api/outside-processing/orders` — Create OP order (draft)
- `POST /api/outside-processing/orders/:id/issue` — Transition draft → issued (creates or links AP PO)
- `POST /api/outside-processing/orders/:id/cancel` — Cancel
- `POST /api/outside-processing/orders/:id/close` — Close completed OP
- `GET /api/outside-processing/orders/:id` — Retrieve with all ship events, return events, reviews, re-identifications
- `GET /api/outside-processing/orders` — List (filters: status, vendor_id, work_order_id, date ranges, service_type)
- `PUT /api/outside-processing/orders/:id` — Update header (while in draft/issued)

### 4.2 Ship/Return Event Endpoints
- `POST /api/outside-processing/orders/:id/ship-events` — Record outbound shipment
- `POST /api/outside-processing/orders/:id/return-events` — Record inbound return
- `GET /api/outside-processing/orders/:id/ship-events` — List
- `GET /api/outside-processing/orders/:id/return-events` — List

### 4.3 Review & Re-ID Endpoints
- `POST /api/outside-processing/orders/:id/reviews` — Record vendor review outcome
- `POST /api/outside-processing/orders/:id/re-identifications` — Record material identity change
- `GET /api/outside-processing/orders/:id/reviews` — List (append-only audit trail)
- `GET /api/outside-processing/orders/:id/re-identifications` — List

### 4.4 Label Endpoints
- `GET /api/outside-processing/status-labels` — Tenant labels over canonical status
- `PUT /api/outside-processing/status-labels/:canonical` — Set label
- `GET /api/outside-processing/service-type-labels` — Tenant service-type codes
- `PUT /api/outside-processing/service-type-labels/:code` — Register/update service-type code + display label

---

## 5. Events Produced & Consumed

Platform envelope: `event_id`, `occurred_at`, `tenant_id`, `source_module` (= `"outside-processing"`), `source_version`, `correlation_id`, `causation_id`, `payload`.

### 5.1 Events Produced

| Event name | Trigger | Key payload |
|------------|---------|-------------|
| `outside_processing.order.created.v1` | OP order created | `op_order_id`, `op_order_number`, `vendor_id`, `service_type`, `work_order_id` |
| `outside_processing.order.issued.v1` | Order issued (draft → issued) | `op_order_id`, `purchase_order_id` (if one was created) |
| `outside_processing.shipment.requested.v1` | Triggered on ship-event creation to notify Shipping-Receiving | `op_order_id`, `ship_event_id`, `vendor_id`, `quantity_shipped`, `lot_number`, `part_number` |
| `outside_processing.shipped.v1` | Ship event recorded (or `shipped_to_vendor` status reached) | `op_order_id`, `ship_event_id`, `quantity_shipped`, `ship_date` |
| `outside_processing.returned.v1` | Return event recorded | `op_order_id`, `return_event_id`, `quantity_received`, `condition`, `received_date` |
| `outside_processing.review.completed.v1` | Vendor review recorded | `op_order_id`, `review_id`, `outcome`, `reviewed_at` |
| `outside_processing.re_identification.recorded.v1` | Material identity changed | `op_order_id`, `old_part_number`, `new_part_number`, `performed_at` |
| `outside_processing.order.closed.v1` | OP order closed | `op_order_id`, `closed_at`, `final_accepted_qty` |
| `outside_processing.order.cancelled.v1` | OP order cancelled | `op_order_id`, `reason` |

### 5.2 Events Consumed

| Event name | Source | Behavior |
|------------|--------|----------|
| `shipping_receiving.shipment.shipped.v1` | Shipping-Receiving | If shipment was created for an OP ship-request, confirm ship-event and advance OP status to `shipped_to_vendor` |
| `shipping_receiving.shipment.received.v1` | Shipping-Receiving | If an inbound shipment references an OP order, create a matching return-event stub; operator completes details |
| `ap.po.approved.v1` | AP | If PO was created for an OP order, mark OP as `issued` (if not already) |
| `ap.po.closed.v1` | AP | Log for audit; doesn't change OP state |
| `inventory.lot.split.v1` | Inventory | If a split lot is linked to an OP order, update OP order's lot reference |

---

## 6. State Machines

### 6.1 OP Order Lifecycle
```
draft ──> issued ──> shipped_to_vendor ──> at_vendor ──> returned ──> review_in_progress ──┬──> closed
   │         │              │                  │            │                               │
   └───> cancelled      cancelled         cancelled     cancelled                   back to at_vendor
                                                                                    (if review outcome = rejected
                                                                                     and vendor will rework)
```
Terminal: `closed`, `cancelled`.

**Transition rules:**
- `draft → issued` allowed only when vendor_id + service_type + quantity_sent are set.
- `issued → shipped_to_vendor` triggered by first ship-event (partial shipments allowed; status advances on first ship).
- `shipped_to_vendor → at_vendor` is automatic when all `quantity_sent` has been shipped OR explicit operator action.
- `at_vendor → returned` triggered by first return-event (again, partial returns allowed).
- `returned → review_in_progress` triggered on first review record creation.
- `review_in_progress → closed` when review outcome = accepted or conditional (with conditions logged).
- `review_in_progress → at_vendor` when review outcome = rejected AND operator chooses to send back to vendor; logged as a second round.
- `* → cancelled` from any non-terminal state (audit trail kept).

### 6.2 Return Event Condition (canonical)
`good`, `damaged`, `discrepancy`

### 6.3 Review Outcome (canonical)
`accepted`, `rejected`, `conditional`

---

## 7. Security & Tenant Isolation

- Shared DB, row-level isolation by `tenant_id`.
- Role gates: `outside_processing:order:issue`, `outside_processing:order:cancel`, `outside_processing:order:close`, `outside_processing:review:record`, `outside_processing:labels:edit`.
- No PII beyond user IDs. No PCI, no financial-instrument data.
- Cost fields (`estimated_cost_cents`, `actual_cost_cents`) are non-sensitive; role-gate read if tenants require.

---

## 8. Required Invariants

1. **Cannot issue without vendor and quantity.** `draft → issued` requires vendor_id + service_type + quantity_sent > 0.
2. **Ship quantities bounded by order quantity.** `SUM(op_ship_events.quantity_shipped) <= op_orders.quantity_sent`.
3. **Return quantities bounded by shipped quantities.** `SUM(op_return_events.quantity_received) <= SUM(op_ship_events.quantity_shipped)`.
4. **Review follows return.** Cannot record a review before at least one return event exists for the order.
5. **Review records append-only.** Corrections create a new review record with a new timestamp. No edits or deletes on `op_vendor_reviews`.
6. **Re-identification requires return event.** Material identity can only change after material comes back.
7. **Work order consistency.** `work_order_id` must reference a platform Production work order that exists and belongs to the same tenant.
8. **Tenant isolation cross-table.** All joined tables share `tenant_id`.
9. **Canonical statuses platform-owned.** Tenants rename via `op_status_labels` but cannot add/remove canonical values.
10. **Events carry canonical codes only.** Downstream modules match on canonical `status`, `condition`, `outcome`, never tenant display labels.

---

## 9. Cross-module integration notes

- **AP:** OP creates or references an AP PO for the vendor. AP owns PO lifecycle; OP tracks operational (shipped / returned / reviewed) lifecycle.
- **Shipping-Receiving:** OP emits `shipment.requested.v1` for outbound to vendor. Shipping-Receiving owns carrier interactions. Inbound returns from vendor arrive via Shipping-Receiving's normal inbound receiving flow, with source-ref back to the OP order.
- **Inventory:** Lots in-transit-out are tracked in Inventory via a standard "in-transit-out" location or status. OP references `lot_id` but doesn't own lot state.
- **Production (when source is work_order):** OP delays the source work order's operation clock — while at-vendor, the operation is effectively paused. Production may react to `outside_processing.shipped.v1` and `outside_processing.returned.v1` to hold/resume operations.
- **Quality-Inspection (optional):** Verticals that want formal inspection on returned material can trigger a Quality-Inspection plan on `outside_processing.returned.v1`. OP's lightweight review record is for operational tracking; formal inspection is a separate workflow.

---

## 10. What stays in Fireproof (aerospace overlay)

Fireproof runs an overlay service that:
- Subscribes to `outside_processing.returned.v1` and `outside_processing.review.completed.v1`
- Stores AS9100-specific metadata: cert-of-conformance document reference, AS-clause citation, formal attestation text, aerospace-specific process-spec linkage
- Emits Fireproof-specific events for AS9100 traceability (e.g. linkage to FAI, CofC acceptance sign-off chain)
- Enforces aerospace-specific rules like "cert review must cite clause X" that don't apply to other verticals

Platform OP records only the generic `cert_ref` (opaque string) and `outcome` (accepted/rejected/conditional). The aerospace-specific attestation text and AS clause mapping live in Fireproof's overlay table.

---

## 11. Open questions

- **PO coupling strength.** Should OP require a PO before `issued`, or allow `issued` without PO (for services billed on a different schedule)? Recommend: PO optional — not every vertical pre-issues a PO for every OP. AP integration is a nice-to-have, not a gate.
- **Partial return with mixed condition.** If a return arrives with some parts good and some damaged, does that split into two return events? Recommend: yes, one event per condition group, so review can target specifically.
- **Rejected-and-returned-to-vendor cycle count.** Should there be a configurable max number of review→back-to-vendor cycles before auto-escalate? Recommend: no limit in platform; verticals can monitor via events and escalate in overlay.
- **`service_type` as canonical enum vs. open string.** Current draft: open string with optional tenant labels. Alternative: constrained enum per vertical. Recommend open string for initial version — verticals self-regulate.
- **Ownership of re-identification.** When part identity changes (e.g. raw material becomes heat-treated material with a new part number), should Inventory create a new lot or update the existing lot? Recommend: Inventory creates a child lot, with lot_genealogy recording parent→child. OP's re-identification record is the trigger; Inventory owns the lot mechanics.
- **Cost reconciliation.** `actual_cost_cents` on OP may differ from AP bill (freight, scrap charges). Should OP reconcile against AP on bill-receipt? Defer — not an MVP need.

---

## 12. Migration notes (from Fireproof)

- Fireproof's `outside_processing/` module (~1,640 LOC) consolidates into this platform module.
- Fireproof's `ncr_id` field (links OP to NCR when vendor work fails) is removed — NCR stays in Fireproof, so the back-link lives in Fireproof's NCR table, not on platform OP.
- Fireproof's `cert_accepted`/`cert_accepted_by`/`cert_accepted_at` header fields are superseded by platform's `op_vendor_reviews` table records; Fireproof's overlay service maintains AS9100-specific additional attestation state.
- Monetary fields convert from `BigDecimal` / `f64` to integer `*_cents`.
- `work_order_id` remains a direct FK to Production work orders.
- Sample data only — drop Fireproof tables, create fresh schema, Fireproof rewires to typed client (`platform_client_outside_processing::*`).
