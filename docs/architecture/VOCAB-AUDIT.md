# Platform Vocabulary & Naming Divergence Audit

**Bead:** bd-sfna4  
**Audit date:** 2026-04-19  
**Methodology:** All facts grepped from source files. Zero reliance on spec documents —
specs are checked against code, not treated as truth. Every claim cites a file:line.

---

## 1. Current-State Table

### 1.1 URL Prefixes (all modules, from `main.rs` or `http/mod.rs`)

| Module | Actual mount prefix | Source |
|--------|--------------------|-------------------------------------------------|
| `ap` | `/api/ap/` | `modules/ap/src/main.rs:47` |
| `ar` | `/api/ar/` | `modules/ar/src/http/tax_reports.rs:65` |
| `bom` | `/api/bom/` | `modules/bom/src/main.rs:160` |
| `consolidation` | (no routes) | — |
| `crm-pipeline` | `/api/crm-pipeline/` | `modules/crm-pipeline/src/main.rs:55` |
| `customer-complaints` | `/api/customer-complaints/` | `modules/customer-complaints/src/main.rs` |
| `customer-portal` | (no routes) | — |
| `fixed-assets` | `/api/fixed-assets/` | `modules/fixed-assets/src/main.rs` |
| `gl` | `/api/gl/` | `modules/gl/src/main.rs:120` |
| `integrations` | `/api/integrations/` | `modules/integrations/src/http/external_refs.rs:78` |
| `inventory` | `/api/inventory/` | `modules/inventory/src/main.rs` |
| `maintenance` | `/api/maintenance/` | `modules/maintenance/src/main.rs:178` |
| `notifications` | (event-driven only) | — |
| `numbering` | `/allocate`, `/confirm`, `/policies/{entity}` | `modules/numbering/src/main.rs:35–46` |
| `outside-processing` | `/api/outside-processing/` | `modules/outside-processing/src/main.rs` |
| `party` | `/api/party/` | `modules/party/src/http/mod.rs:21` |
| `payments` | (no HTTP routes) | — |
| `pdf-editor` | `/api/pdf/` | `modules/pdf-editor/src/main.rs` |
| `production` | `/api/production/` | `modules/production/src/http/time_entries.rs:26` |
| `quality-inspection` | `/api/quality-inspection/` | `modules/quality-inspection/src/main.rs` |
| `reporting` | `/api/reporting/` | `modules/reporting/src/main.rs` |
| `sales-orders` | `/api/so/` | `modules/sales-orders/src/main.rs:45` |
| `shipping-receiving` | `/api/shipping-receiving/` | `modules/shipping-receiving/src/routes.rs:23` |
| `shop-floor-gates` | `/api/sfg/` | `modules/shop-floor-gates/src/main.rs:43` |
| `subscriptions` | `/api/bill-runs/` | `modules/subscriptions/src/http/mod.rs:63` |
| `timekeeping` | `/api/timekeeping/` | `modules/timekeeping/src/http/mod.rs:165` |
| `treasury` | `/api/treasury/` | `modules/treasury/src/main.rs` |
| `ttp` | (internal/event-driven) | — |
| `workflow` | `/api/workflow/` | `modules/workflow/src/main.rs:31` |
| `workforce-competence` | `/api/workforce-competence/` | `modules/workforce-competence/src/main.rs` |

### 1.2 Event Subjects Emitted (code-derived, by module)

#### `production`
Source: `modules/production/src/events/mod.rs:38–61`

| Event type string | Wire subject | Notes |
|-------------------|-------------|-------|
| `production.work_order_created` | direct (no prefix-adding publisher) | underscore join |
| `production.work_order_released` | direct | underscore join |
| `production.work_order_closed` | direct | underscore join |
| `production.component_issue.requested` | direct | **dot** between entity and action |
| `production.component_issued` | direct | underscore join |
| `production.operation_started` | direct | underscore join |
| `production.operation_completed` | direct | underscore join |
| `production.fg_received` | direct | underscore join |
| `production.fg_receipt.requested` | direct | **dot** between entity and action |
| `production.workcenter_created` | direct | underscore join |
| `production.workcenter_updated` | direct | underscore join |
| `production.workcenter_deactivated` | direct | underscore join |
| `production.routing_created` | direct | underscore join |
| `production.routing_updated` | direct | underscore join |
| `production.routing_released` | direct | underscore join |
| `production.time_entry_created` | direct | underscore join |
| `production.time_entry_stopped` | direct | underscore join |
| `production.time_entry_approved` | direct | underscore join |
| `production.downtime.started` | direct | **dot** between entity and action |
| `production.downtime.ended` | direct | **dot** between entity and action |
| `production.cost_posted` | direct | underscore join |
| `production.work_order_cost_finalized` | direct | underscore join |

#### `sales-orders`
Source: `modules/sales-orders/src/events/`

| Event type string | Wire subject |
|-------------------|-------------|
| `sales_orders.order_created` | direct |
| `sales_orders.order_booked` | direct |
| `sales_orders.order_cancelled` | direct |
| `sales_orders.order_shipped` | direct |
| `sales_orders.order_closed` | direct |
| `sales_orders.blanket_activated` | direct |
| `sales_orders.blanket_expired` | direct |
| `sales_orders.blanket_cancelled` | direct |
| `sales_orders.release_created` | direct |
| `sales_orders.reservation_requested` | direct |
| `sales_orders.shipment_requested` | direct |
| `sales_orders.invoice_requested` | direct |

Sources: `modules/sales-orders/src/events/orders.rs:15–19`, `modules/sales-orders/src/events/blankets.rs:12–116`, `modules/sales-orders/src/events/reservation.rs:13–15`

#### `crm-pipeline`
Source: `modules/crm-pipeline/src/events/lead.rs:13–15`, `modules/crm-pipeline/src/events/opportunity.rs:10–13`, `modules/crm-pipeline/src/events/activity.rs:10–12`

| Event type string | Wire subject |
|-------------------|-------------|
| `crm_pipeline.lead_created` | direct |
| `crm_pipeline.lead_status_changed` | direct |
| `crm_pipeline.lead_converted` | direct |
| `crm_pipeline.opportunity_created` | direct |
| `crm_pipeline.opportunity_stage_advanced` | direct |
| `crm_pipeline.opportunity_closed_won` | direct |
| `crm_pipeline.opportunity_closed_lost` | direct |
| `crm_pipeline.activity_logged` | direct |
| `crm_pipeline.activity_completed` | direct |
| `crm_pipeline.activity_overdue` | direct |

#### `outside-processing`
Source: `modules/outside-processing/src/events/`

| Event type string | Wire subject |
|-------------------|-------------|
| `outside_processing.order_created` | direct |
| `outside_processing.order_issued` | direct |
| `outside_processing.order_closed` | direct |
| `outside_processing.order_cancelled` | direct |
| `outside_processing.shipment_requested` | direct |
| `outside_processing.shipped` | direct |
| `outside_processing.returned` | direct |
| `outside_processing.review_completed` | direct |
| `outside_processing.re_identification_recorded` | direct |

Source: `modules/outside-processing/src/events/mod.rs:12–20`

#### `shop-floor-gates`
Source: `modules/shop-floor-gates/src/events/mod.rs:6–15`

| Event type string | Wire subject |
|-------------------|-------------|
| `shop_floor_gates.hold_placed` | direct |
| `shop_floor_gates.hold_released` | direct |
| `shop_floor_gates.hold_cancelled` | direct |
| `shop_floor_gates.handoff_initiated` | direct |
| `shop_floor_gates.handoff_accepted` | direct |
| `shop_floor_gates.handoff_rejected` | direct |
| `shop_floor_gates.handoff_cancelled` | direct |
| `shop_floor_gates.verification_operator_confirmed` | direct |
| `shop_floor_gates.verification_completed` | direct |
| `shop_floor_gates.signoff_recorded` | direct |

#### `shipping-receiving`
Source: `modules/shipping-receiving/src/events/mod.rs`

| Event type string | Wire subject (published direct) |
|-------------------|---------------------------------|
| `shipping_receiving.shipment_created` | direct |
| `shipping_receiving.shipment_status_changed` | direct |
| `shipping_receiving.inbound_closed` | direct |
| `shipping_receiving.outbound_shipped` | direct |
| `shipping_receiving.outbound_delivered` | direct |

Source: `modules/shipping-receiving/src/events/contracts.rs:32–44`

#### `inventory` (partial — showing inconsistent `.v1` events)
Source: `modules/inventory/src/events/`

Most events: no `.v1` suffix (e.g., `inventory.item_received` at `contracts.rs:31`, `inventory.adjusted` at `contracts.rs:38`).  
Six events carry `.v1` suffix:

| Event type string | Source |
|-------------------|--------|
| `inventory.lot_split.v1` | `events/lot_split.rs:15` |
| `inventory.lot_merged.v1` | `events/lot_merged.rs:15` |
| `inventory.expiry_set.v1` | `events/expiry_set.rs:10` |
| `inventory.expiry_alert.v1` | `events/expiry_alert.rs:10` |
| `inventory.classification_assigned.v1` | `events/classification_assigned.rs:14` |
| `inventory.label_generated.v1` | `events/label_generated.rs:15` |

### 1.3 Event Subjects Subscribed To (code-derived)

| Module | Subject subscribed | Source |
|--------|--------------------|--------|
| `crm-pipeline` | `party.deactivated` | `consumers/party_deactivated.rs:48` |
| `crm-pipeline` | `party.events.contact.deactivated` | `consumers/contact_deactivated.rs:49` |
| `crm-pipeline` | `sales_orders.order_booked` | `consumers/order_booked.rs:53` |
| `crm-pipeline` | `ar.customer.created.v1` | `consumers/customer_created.rs:30` |
| `outside-processing` | `ap.po_approved` | `consumers/ap_po_approved.rs:15` |
| `outside-processing` | `ap.po_closed` | `consumers/ap_po_closed.rs:15` |
| `outside-processing` | `inventory.lot_split` | `consumers/inventory_lot_split.rs:16` |
| `outside-processing` | `shipping_receiving.shipment_received` | `consumers/shipment_received.rs:17` |
| `outside-processing` | `shipping_receiving.shipment_shipped` | `consumers/shipment_shipped.rs:17` |
| `sales-orders` | `shipping_receiving.shipment_shipped.v1` | `consumers/shipment_shipped.rs:18` |
| `sales-orders` | `ar.invoice_issued.v1` | `consumers/invoice_issued.rs:13` |
| `shipping-receiving` | `ap.events.ap.po_approved` | `main.rs:124` (SDK `.consumer()` call) |
| `shipping-receiving` | `sales.so.released` | `consumers/so_released.rs:21` (**dead code**) |
| `shop-floor-gates` | `production.work_order_closed` | `consumers/work_order_closed.rs:12` |
| `shop-floor-gates` | `production.operation_completed` | `consumers/operation_completed.rs:12` |

---

## 2. Divergence List

### 2.1 URL Prefix Divergence

**D-URL-1: `sales-orders` uses abbreviated prefix `/api/so/`**

- Code: `/api/so/orders` (`modules/sales-orders/src/main.rs:45`)
- Client library: `/api/so/orders` (`clients/sales-orders/src/orders.rs:24`)
- Spec says: `/api/sales-orders/orders` (`docs/architecture/SALES-ORDERS-MODULE-SPEC.md:70`)
- Severity: **BREAKING** — all consumers using the typed client are already on `/api/so/`; renaming the server prefix breaks them unless client is updated atomically.

**D-URL-2: `shop-floor-gates` uses abbreviated prefix `/api/sfg/`**

- Code: `/api/sfg/holds` (`modules/shop-floor-gates/src/main.rs:43`)
- Client library: `/api/sfg/holds` (`clients/shop-floor-gates/src/lib.rs:26`)
- Spec says: `/api/shop-floor-gates/holds` (`docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md:69`)
- Severity: **BREAKING** — same impact as D-URL-1.

**D-URL-3: `subscriptions` exposes `/api/bill-runs/` not `/api/subscriptions/`**

- Code: `/api/bill-runs/execute` (`modules/subscriptions/src/http/mod.rs:63`)
- No spec covers subscriptions route prefix.
- Severity: **COSMETIC** for now; but diverges from the full-name convention used by all other modules.

**D-URL-4: `numbering` has no `/api/numbering/` prefix**

- Code: `/allocate`, `/confirm`, `/policies/{entity}` (`modules/numbering/src/main.rs:35–46`)
- Client: `/allocate` (`clients/numbering/src/numbering.rs:24`)
- No spec covers prefix.
- Severity: **COSMETIC** — module-to-module calls use the typed client which already encodes these paths. But the pattern is invisible to API consumers expecting `/api/numbering/`.

**D-URL-5: `ap`, `ar`, `gl`, `bom`, `pdf-editor` use 2-4 letter abbreviations**

These predate the full-name convention. All are consistent internally (server + typed client both use the abbreviated prefix). No spec document prescribes a prefix for them.
- Severity: **COSMETIC** — consistent within each module; no immediate breakage.

---

### 2.2 Event Subject Naming: Production Module Inconsistency

**D-SUBJ-1: Production mixes underscore-join and dot-separated subjects**

Four subjects in `production` use a dot between the noun phrase and the action verb instead of an underscore:

| Subject | Pattern |
|---------|---------|
| `production.work_order_created` | underscore ✓ |
| `production.component_issue.requested` | **dot** |
| `production.fg_receipt.requested` | **dot** |
| `production.downtime.started` | **dot** |
| `production.downtime.ended` | **dot** |

Source: `modules/production/src/events/mod.rs:38–61`

The underscore pattern (`module.entity_action`) is used by all other modules (`crm_pipeline`, `sales_orders`, `outside_processing`, `shop_floor_gates`, `shipping_receiving`). The four dot-separated subjects follow the `ar.payment.collection.requested` pattern from the EVENT-TAXONOMY doc, which acknowledges both patterns but doesn't mandate either.

Severity: **COSMETIC** — these events are currently consumed only by `shop-floor-gates` (`production.operation_completed`) and `production` itself internally. No live consumer subscribes to `production.component_issue.requested` or `production.fg_receipt.requested` via NATS; they are consumed by other production domain services directly.

---

### 2.3 Event-Name Drift (spec says X, code emits Y)

**D-SPEC-1: SALES-ORDERS spec uses `.v1` suffix and dot-separator in event names**

Spec produces `sales_orders.order.created.v1` (`docs/architecture/SALES-ORDERS-MODULE-SPEC.md:106`); code emits `sales_orders.order_created` (`modules/sales-orders/src/events/orders.rs:15`). The drift is systemic across all 12 SO event types. The spec was authored before the platform settled on the underscore-join pattern.

Severity: **COSMETIC** for the emitter side. The spec is wrong; code is correct.

**D-SPEC-2: OUTSIDE-PROCESSING spec uses `.v1` suffix and dot-separator**

Same pattern as D-SPEC-1 — spec says `outside_processing.order.created.v1`, code emits `outside_processing.order_created` (`modules/outside-processing/src/events/mod.rs:12`). All 9 OP event names are affected.

Severity: **COSMETIC** for the emitter side. Spec is wrong; code is correct.

**D-SPEC-3: CRM-PIPELINE spec uses `.v1` suffix and dot-separator**

Spec says `crm_pipeline.lead.created.v1` (`docs/architecture/CRM-PIPELINE-MODULE-SPEC.md:132`); code emits `crm_pipeline.lead_created` (`modules/crm-pipeline/src/events/lead.rs:13`). All 10 CRM event types are affected.

Severity: **COSMETIC** for the emitter side. Spec is wrong; code is correct.

**D-SPEC-4: SHOP-FLOOR-GATES spec uses `.v1` suffix and dot-separator for emitted events**

Spec says `shop_floor_gates.hold.placed.v1` (`docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md:111`); code emits `shop_floor_gates.hold_placed` (`modules/shop-floor-gates/src/events/mod.rs:6`). All 10 SFG event types are affected.

Severity: **COSMETIC** for the emitter side. Spec is wrong; code is correct.

---

### 2.4 Phantom Events (consumer subscribes to something no emitter produces)

**D-PHANTOM-1: `sales-orders` → `shipping_receiving.shipment_shipped.v1` (SILENT FAILURE)**

- Subscriber: `modules/sales-orders/src/consumers/shipment_shipped.rs:18`
- Actual event emitted by shipping-receiving: `shipping_receiving.outbound_shipped` (no `.v1`, different event name)
- Source: `modules/shipping-receiving/src/events/contracts.rs:41`
- Impact: Sales-Orders never receives shipment confirmation. `order_shipped` event is never emitted. `shipped_qty` on SO lines is never updated.
- Severity: **BREAKING (P0)** — silent data integrity failure. Orders will never transition to shipped status via the event path.

**D-PHANTOM-2: `sales-orders` → `ar.invoice_issued.v1` (SILENT FAILURE)**

- Subscriber: `modules/sales-orders/src/consumers/invoice_issued.rs:13`
- AR does not emit `ar.invoice_issued`. AR emits `ar.invoice_opened` (`modules/ar/src/events/contracts/invoice_lifecycle.rs:21`) on the wire subject `ar.events.ar.invoice_opened`.
- Impact: Sales-Orders never links an AR invoice_id back onto the SO.
- Severity: **BREAKING (P1)** — SO-to-invoice cross-reference is never populated. Reporting across SO + AR is blind.

**D-PHANTOM-3: `outside-processing` → `shipping_receiving.shipment_shipped` (SILENT FAILURE)**

- Subscriber: `modules/outside-processing/src/consumers/shipment_shipped.rs:17`
- Actual event from shipping-receiving: `shipping_receiving.outbound_shipped`
- Source: `modules/shipping-receiving/src/events/contracts.rs:41`
- Impact: OP status never advances to `shipped_to_vendor` via the event path.
- Severity: **BREAKING (P0)** — OP ship-events confirm flow is silently broken.

**D-PHANTOM-4: `outside-processing` → `shipping_receiving.shipment_received` (SILENT FAILURE)**

- Subscriber: `modules/outside-processing/src/consumers/shipment_received.rs:17`
- Actual event from shipping-receiving: `shipping_receiving.inbound_closed`
- Source: `modules/shipping-receiving/src/events/contracts.rs:38`
- Impact: OP return-event stubs are never created when vendor goods arrive.
- Severity: **BREAKING (P0)** — inbound returns from vendor are not linked to OP orders.

**D-PHANTOM-5: `outside-processing` → `inventory.lot_split` vs `inventory.lot_split.v1` (SILENT FAILURE)**

- Subscriber: `modules/outside-processing/src/consumers/inventory_lot_split.rs:16` subscribes to `inventory.lot_split` (no `.v1`)
- Inventory emits `inventory.lot_split.v1` (`modules/inventory/src/events/lot_split.rs:15`)
- Impact: OP order lot references are never updated when a linked lot is split.
- Severity: **BREAKING (P1)** — lot lineage tracking for OP orders is silently broken.

**D-PHANTOM-6: `crm-pipeline` → `ar.customer.created.v1` (PHANTOM EMITTER)**

- Subscriber: `modules/crm-pipeline/src/consumers/customer_created.rs:30`
- AR does not emit any `ar.customer.created` event. No emitter exists anywhere in the codebase.
- Severity: **INVESTIGATION NEEDED** — the consumer is a no-op stub ("future enhancement"), so silent failure has no current data impact. But the event contract it expects (`ar.customer.created`) needs to either be added to AR's emitted surface or the consumer should be removed.

**D-PHANTOM-7: `shop-floor-gates` has no consumer for `production.work_order.cancelled`**

- SHOP-FLOOR-GATES-MODULE-SPEC.md:126 says the module should consume `production.work_order.cancelled.v1` to auto-cancel active holds.
- Production does NOT emit `production.work_order.cancelled` or `production.work_order_cancelled`. There is no cancellation event — only `production.work_order_closed`.
- No consumer for this event exists in the module.
- Severity: **INVESTIGATION NEEDED** — the spec describes a behavior that is not implemented. Either production needs to emit a `work_order_cancelled` event, or the spec is aspirational and holds are never auto-cancelled when a WO is cancelled.

---

### 2.5 Contract Filename vs Event Type

**D-CONTRACT-1: `ar-invoice-issued.v1.json` title does not match filename**

- File: `contracts/events/ar-invoice-issued.v1.json`
- Title field inside: `"ar.invoice_opened"` (this is the actual event type)
- The filename says "issued" but the event is "opened". This predates the event being renamed.
- Severity: **COSMETIC** — confusing to readers and tooling that maps filenames to subjects.

**D-CONTRACT-2: Contract files use dashes; subjects use underscores (by design, undocumented)**

- Filenames: `crm-pipeline-activity-logged.v1.json` (dashes everywhere)
- Wire subjects: `crm_pipeline.activity_logged` (module prefix underscore, word-join underscore)
- This is intentional (dashes are idiomatic for filenames; underscores for code identifiers). However, no document states this explicitly. EVENT-TAXONOMY.md is silent on filename convention.
- Severity: **COSMETIC** — add a note to EVENT-TAXONOMY.md.

---

### 2.6 Dead Consumer Code

**D-DEAD-1: `shipping-receiving` — `SUBJECT_PO_APPROVED = "ap.po.approved"` is stale dead code**

- `modules/shipping-receiving/src/consumers/po_approved.rs:22`: `pub const SUBJECT_PO_APPROVED: &str = "ap.po.approved"`
- This constant is only used by `start_po_approved_consumer()`, which is exported from `consumers/mod.rs:4` but never called from `main.rs`.
- The actual live subscription is wired via the SDK at `main.rs:124`: `.consumer("ap.events.ap.po_approved", on_po_approved)`.
- AP emits `ap.po_approved`; the SDK-based double-prefix publisher sends it on `ap.events.ap.po_approved`.
- The old constant `"ap.po.approved"` is wrong AND dead.
- Severity: **COSMETIC** — no runtime impact; misleading to readers.

**D-DEAD-2: `shipping-receiving/src/consumers/so_released.rs` — dead file**

- `SUBJECT_SO_RELEASED = "sales.so.released"` at `so_released.rs:21`
- `start_so_released_consumer()` is exported from `consumers/mod.rs:5` but never called from `main.rs`.
- No sales module emits `sales.so.released`. This consumer was written for a planned module that was not built.
- `REVISIONS.md:2.2.7` intended to remove this as part of `bd-thx8s` but only removed it from `main.rs`. The `consumers/so_released.rs` and `consumers/po_approved.rs` files remain.
- Severity: **COSMETIC** — dead code, no runtime impact.

---

### 2.7 Inventory `.v1` Inconsistency

**D-INV-1: Six inventory events carry `.v1` suffix; the other ~15 do not**

- Without `.v1`: `inventory.item_received`, `inventory.item_issued`, `inventory.adjusted`, `inventory.transfer_completed`, `inventory.item_reserved`, `inventory.reservation_released`, `inventory.valuation_run_completed`, etc.
- With `.v1`: `inventory.lot_split.v1`, `inventory.lot_merged.v1`, `inventory.expiry_set.v1`, `inventory.expiry_alert.v1`, `inventory.classification_assigned.v1`, `inventory.label_generated.v1`
- The `.v1` events were added later and accidentally included the version suffix in the event_type string. The platform convention (confirmed by `crm-pipeline/src/events/mod.rs:7` comment and the EVENT-TAXONOMY doc) is that `.v1` belongs in contract *filenames*, not in subject strings.
- Severity: **BREAKING (P1)** — `outside-processing` subscribes to `inventory.lot_split` (without `.v1`) and misses `inventory.lot_split.v1` (D-PHANTOM-5). Any future consumer of the other five events faces the same mismatch.

---

## 3. Proposed Platform Conventions

### C-1: URL Prefix — full module name, no abbreviations

**Rule:** All HTTP mounts use `/api/{module-name}/` where `{module-name}` is the kebab-case directory name under `modules/`.

- `sales-orders` → `/api/sales-orders/`
- `shop-floor-gates` → `/api/sfg/` **→** `/api/shop-floor-gates/`
- `ap` stays `/api/ap/` — two-letter modules are grandfathered; a rename would break all external AP integrations with no benefit proportional to the risk.
- `subscriptions` → `/api/subscriptions/` (currently `/api/bill-runs/`)
- `numbering` → `/api/numbering/allocate`, `/api/numbering/policies/{entity}` (add prefix)

**Exception:** `ap`, `ar`, `gl`, `bom` are well-known industry abbreviations and their prefixes are shared with external clients. Propose no change to these.

### C-2: Event subjects — `{module_snake}.{entity_action}` with no `.v1` suffix

**Rule:** NATS wire subjects use the event_type directly; event_type format is `{module_snake}.{entity_action}` where:
- `{module_snake}` = underscore version of the module directory (e.g., `crm_pipeline`, `sales_orders`, `shop_floor_gates`)
- `{entity_action}` = underscore-joined noun phrase and verb (e.g., `order_booked`, `hold_placed`, `work_order_closed`)
- No `.v1` suffix — versioning lives in contract filenames and `schema_version` fields only

**Consequence for production:** the four dot-separated subjects (`component_issue.requested`, `fg_receipt.requested`, `downtime.started`, `downtime.ended`) should migrate to `component_issue_requested`, `fg_receipt_requested`, `downtime_started`, `downtime_ended`. These events are currently not subscribed to by any external consumer (verified by grep), so the rename is lower-risk than SO or SFG URL renames.

**Consequence for inventory:** remove `.v1` from the six affected event type strings and emit on the clean subject. Requires coordinating with any subscriber (currently only `outside-processing` for `lot_split`, and no external subscribers for the others).

### C-3: Contract filenames — `{module-kebab}-{entity-action}.v1.json`

**Rule:** filenames use dashes throughout; `.v1` suffix marks schema major version. This is already the practice — just needs to be stated in EVENT-TAXONOMY.md.

**Exception to fix:** rename `ar-invoice-issued.v1.json` → `ar-invoice-opened.v1.json` and update the `$id` field inside. The current mismatch between filename ("issued") and title ("opened") is a reader trap.

### C-4: Spec documents — code is truth; specs must reference wire subjects

**Rule:** when a spec references an event (consumed or produced), it must cite the exact event_type string as it appears in the code, not a hypothetical `.v1`-suffixed version. All spec documents listing events should include a line like `> Source of truth: \`modules/{module}/src/events/\`` so readers know where to verify.

---

## 4. Follow-Up Bead Recommendations

### FIX-1: Rename `sales-orders` consumer subscription (D-PHANTOM-1)
**Shape:** 1 bead  
**Files:** `modules/sales-orders/src/consumers/shipment_shipped.rs:18`  
**Change:** `"shipping_receiving.shipment_shipped.v1"` → `"shipping_receiving.outbound_shipped"`  
**Severity:** BREAKING (P0) — currently SO `order_shipped` is never emitted  
**Note:** Also verify what `SO: shipment_shipped consumer` does with the payload — the event payload shape of `outbound_shipped` may differ from what the consumer expects.

### FIX-2: Rename `sales-orders` AR consumer subscription (D-PHANTOM-2)
**Shape:** 1 bead  
**Files:** `modules/sales-orders/src/consumers/invoice_issued.rs:13`  
**Change:** `"ar.invoice_issued.v1"` → `"ar.events.ar.invoice_opened"` (the actual wire subject AR publishes to)  
**Severity:** BREAKING (P1) — SO-to-invoice link is never set

### FIX-3: Rename `outside-processing` shipment consumers (D-PHANTOM-3, D-PHANTOM-4)
**Shape:** 1 bead (both are same module, same fix pattern)  
**Files:**
- `modules/outside-processing/src/consumers/shipment_shipped.rs:17`: `"shipping_receiving.shipment_shipped"` → `"shipping_receiving.outbound_shipped"`
- `modules/outside-processing/src/consumers/shipment_received.rs:17`: `"shipping_receiving.shipment_received"` → `"shipping_receiving.inbound_closed"`  
**Severity:** BREAKING (P0) for both consumers — OP ship/return flows are silently broken

### FIX-4: Fix `outside-processing` lot_split subscription (D-PHANTOM-5 / D-INV-1)
**Shape:** 1 bead (choose one of two approaches)  
**Option A (preferred):** Remove `.v1` from `inventory.lot_split.v1` and the five other inventory events. Update `outside-processing` consumer to `"inventory.lot_split"`.  
**Option B:** Add `.v1` to `outside-processing` consumer. Does not fix the broader inventory inconsistency.  
**Files:** `modules/inventory/src/events/lot_split.rs:15` + 5 other inventory event files, `modules/outside-processing/src/consumers/inventory_lot_split.rs:16`  
**Severity:** BREAKING (P1) — lot tracking for OP is silent failure

### FIX-5: Remove dead SR consumer files (D-DEAD-1, D-DEAD-2)
**Shape:** 1 bead  
**Files:**
- `modules/shipping-receiving/src/consumers/po_approved.rs` — delete `SUBJECT_PO_APPROVED` constant and `start_po_approved_consumer` function (the live path is the SDK consumer in `main.rs:124`)
- `modules/shipping-receiving/src/consumers/so_released.rs` — delete entire file
- `modules/shipping-receiving/src/consumers/mod.rs` — remove exports of the deleted functions  
**Severity:** COSMETIC — no runtime impact; reduces confusion

### FIX-6: Rename `ar-invoice-issued.v1.json` (D-CONTRACT-1)
**Shape:** child of contract-cleanup bead  
**Files:** `contracts/events/ar-invoice-issued.v1.json`, `docs/event-contract-audit.md`  
**Change:** rename file → `ar-invoice-opened.v1.json`, update `$id` field inside  
**Severity:** COSMETIC

### FIX-7: Rename `sales-orders` URL prefix `/api/so/` → `/api/sales-orders/` (D-URL-1)
**Shape:** 1 bead, coordinate with all vertical teams  
**Files:** `modules/sales-orders/src/main.rs`, `clients/sales-orders/src/` (all route strings), any vertical that calls SO directly  
**Severity:** BREAKING — must be done atomically with client update; requires a sprint window  
**Defer until after current sprint.**

### FIX-8: Rename `shop-floor-gates` URL prefix `/api/sfg/` → `/api/shop-floor-gates/` (D-URL-2)
**Shape:** 1 bead, same pattern as FIX-7  
**Files:** `modules/shop-floor-gates/src/main.rs`, `clients/shop-floor-gates/src/lib.rs` (all 10+ route strings)  
**Severity:** BREAKING  
**Defer until after current sprint.**

### FIX-9: Update all 4 spec documents to use code-accurate event names (D-SPEC-1 through D-SPEC-4)
**Shape:** 1 bead (docs-only, no code)  
**Files:**
- `docs/architecture/SALES-ORDERS-MODULE-SPEC.md` §5.1, §5.2
- `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md` §5.1, §5.2
- `docs/architecture/CRM-PIPELINE-MODULE-SPEC.md` §5.1, §5.2
- `docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md` §5.1, §5.2  
**Change:** replace all `module.entity.action.v1` with `module.entity_action`; replace spec-only events that don't exist in code with `TBD (not yet implemented)`  
**Severity:** COSMETIC — docs-only; prevents future phantom subscriptions from being built against bad spec

### FIX-10: Investigate `crm-pipeline` `ar.customer.created.v1` consumer (D-PHANTOM-6)
**Shape:** investigation bead → either add the event to AR or delete the CRM stub  
**Files:** `modules/crm-pipeline/src/consumers/customer_created.rs`, `modules/ar/src/events/`  
**Severity:** INVESTIGATION NEEDED — stub is no-op today but the unanswered question is whether the CRM-to-AR link is actually wanted

### FIX-11: Investigate `shop-floor-gates` missing WO-cancelled consumer (D-PHANTOM-7)
**Shape:** investigation bead  
**Question:** should production emit `production.work_order_cancelled`? Or should SFG hold auto-cancel be triggered differently?  
**Severity:** INVESTIGATION NEEDED — spec describes behavior that has no implementation

### FIX-12: Document contract filename convention in EVENT-TAXONOMY.md (D-CONTRACT-2)
**Shape:** 1 line in existing spec, negligible bead  
**File:** `docs/architecture/EVENT-TAXONOMY.md`  
**Severity:** COSMETIC

---

## Appendix: CRM Consumer Accuracy Summary

The CRM spec (§5.2) says it subscribes to `party.party.deactivated.v1` and `party.contact.deactivated.v1`. The code is more accurate:

| Spec says | Code subscribes to | Actual wire subject from emitter |
|-----------|--------------------|----------------------------------|
| `party.party.deactivated.v1` | `party.deactivated` (`party_deactivated.rs:48`) | `party.deactivated` (`modules/party/src/events/party.rs:16`) ✓ |
| `party.contact.deactivated.v1` | `party.events.contact.deactivated` (`contact_deactivated.rs:49`) | `party.events.contact.deactivated` (`modules/party/src/events/contact.rs:17`) ✓ |
| `sales_orders.order.booked.v1` | `sales_orders.order_booked` (`order_booked.rs:53`) | `sales_orders.order_booked` (`modules/sales-orders/src/events/orders.rs:16`) ✓ |
| `ar.customer.created.v1` | `ar.customer.created.v1` (`customer_created.rs:30`) | **(none)** — phantom emitter |

The first three CRM subscriptions work correctly despite the spec being wrong. The fourth (D-PHANTOM-6) subscribes to a phantom event.
