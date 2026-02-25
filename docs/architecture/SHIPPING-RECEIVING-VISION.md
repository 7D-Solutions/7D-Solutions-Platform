# Shipping-Receiving Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: v0.1.0 In Progress

---

## 1. What Are We Building

Shipping-Receiving is a unified logistics module that tracks and executes physical inbound receiving and outbound shipping for tenant-owned operations. It manages shipments end-to-end with clear state machines, links shipments to upstream business documents (POs, sales orders, transfer requests), and triggers the inventory stock ledger at the correct lifecycle points (inbound close → receipts; outbound ship → issues). It also provides operational dashboards and an immutable status history so the platform can answer "what's in transit, what's late, what shipped, and what was received" without guesswork.

---

## 2. Domain Authority

### Module Authority Matrix Entry

| Module | Owns | May mutate | Produces | Consumes |
|---|---|---|---|---|
| `shipping-receiving-rs` | shipments, shipment_lines, shipment_status_history, shipping-receiving outbox & idempotency keys | inventory stock ledger via Inventory API (receipts/issues) for shipment lifecycle actions; shipment refs on itself only | `shipping_receiving.shipment_created`, `shipping_receiving.shipment_status_changed`, `shipping_receiving.inbound_closed`, `shipping_receiving.outbound_shipped`, `shipping_receiving.outbound_delivered` | `ap.po.approved` (for inbound expected), `sales.so.released` (for outbound created), `inventory.receipt.confirmed` (to update inbound linkage), `inventory.issue.confirmed` (optional) |

**Clarification**: The module does not own carriers; carrier identity is referenced via `party_id` from Party Master. It may read external refs (PO IDs, sales order IDs) but does not mutate those modules' databases.

---

## 3. Domain Ownership

### Domain Ownership Registry Entry

**Module**: `shipping-receiving-rs`
**Module Path**: `modules/shipping-receiving/`
**Database**: `shipping_receiving_db` (tenant-scoped data via `tenant_id` on every table)
**Port**: 8103

### Tables

- `shipments`
- `shipment_lines`
- `shipment_status_history` (append-only)
- `events_outbox` (module outbox for NATS)
- `processed_events` / idempotency table (module idempotency + consumer replay safety)

### Domain Responsibilities

- Manage shipment lifecycle for both directions:
  - **Inbound (Receiving)**: expected → in_transit → arrived → receiving → closed (+ cancelled)
  - **Outbound (Shipping)**: created → picking → packed → shipped → delivered (+ cancelled)
- Enforce invariants:
  - Inbound close: `qty_accepted + qty_rejected == qty_received` (per line)
  - Outbound ship: `qty_shipped > 0` and `qty_shipped <= qty_expected` (per line)
- Provide operational read models:
  - Status history per shipment
  - Dashboard aggregates: open-by-status, overdue, by carrier, by direction
- Trigger inventory movements exactly once at lifecycle boundaries:
  - Inbound close → receipts for accepted quantity
  - Outbound ship → issues for shipped quantity
- Provide traceability links:
  - Inbound to AP Purchase Orders / PO lines
  - Outbound to AR Sales Orders (or other outbound sources) via `source_ref`

### External Dependencies

**Consumes**:
- `ap.purchase_order.approved` (auto-create inbound expected shipment)
- `ar.sales_order.released` (auto-create outbound shipment)
- `inventory.receipt.confirmed` (attach receipt refs / reconcile inbound)
- `inventory.issue.confirmed` (optional; attach issue refs / reconcile outbound)

**Produces**:
- Shipping-receiving shipment lifecycle events (see Event Taxonomy below)

**Calls (HTTP)**:
- Inventory: create receipt / create issue (idempotent, deterministic keys)

**References (Foreign Domain IDs)**:
- Party: `carrier_party_id` (carrier identity)
- AP: `po_id`, `po_line_id` (inbound link)
- AR: `source_ref_type='sales_order'`, `source_ref_id` (outbound link)

---

## 4. Event Taxonomy

Domain name in events: `shipping_receiving` (underscore, matching Rust module naming).

> **Note:** The event domain prefix uses underscores (`shipping_receiving`) rather than hyphens, aligning with the Rust crate name convention. The NATS subject routing uses the same prefix.

### Emitted Events (v0.1.0)

**Shipment lifecycle:**
- `shipping_receiving.shipment_created` — new inbound or outbound shipment
- `shipping_receiving.shipment_status_changed` — any status transition

**Inbound-specific facts:**
- `shipping_receiving.inbound_closed` (includes receipt linkage per line)

**Outbound-specific facts:**
- `shipping_receiving.outbound_shipped` (includes issue linkage per line)
- `shipping_receiving.outbound_delivered`

**Planned (not yet implemented):**
- `shipping_receiving.shipment_cancelled`
- `shipping_receiving.shipment_arrived`

### Payload Expectations

All events are wrapped in `EventEnvelope` and include:
- `tenant_id`
- `correlation_id`, `causation_id`
- `schema_version`
- `replay_safe`
- `mutation_class`

At minimum, payloads include:
- `shipment_id`
- `direction` (inbound | outbound)
- `old_status` / `new_status` where relevant
- Carrier and tracking fields where present

Line-level linkage for inventory refs on closed/shipped events:
- Inbound: `{ line_id, sku, warehouse_id, qty_accepted, receipt_id }`
- Outbound: `{ line_id, sku, warehouse_id, qty_shipped, issue_id }`

---

## 5. State Machines

### Inbound Shipment (Receiving)

**States:** expected, in_transit, arrived, receiving, closed (terminal), cancelled (terminal)

**Allowed transitions:**
- expected → in_transit
- expected → arrived
- in_transit → arrived
- arrived → receiving
- receiving → closed
- expected → cancelled
- in_transit → cancelled
- arrived → cancelled
- receiving → cancelled

**Guards:**
- Entering `arrived` requires `arrived_at` set
- Entering `closed` requires per-line invariant: `accepted + rejected == received`
- `closed` locks edits (lines immutable except read projections)

### Outbound Shipment (Shipping)

**States:** created, picking, packed, shipped, delivered (terminal), cancelled (terminal)

**Allowed transitions:**
- created → picking
- picking → packed
- packed → shipped
- shipped → delivered
- created → cancelled
- picking → cancelled
- packed → cancelled

**Guards:**
- Entering `shipped` requires `qty_shipped > 0` for at least one line and `qty_shipped <= qty_expected` per line
- Shipping is idempotent: cannot ship twice
- `delivered` locks edits (immutable)

---

## 6. Integration Map

### Inventory (required)

**Purpose**: Reflect physical movement into the stock ledger exactly once.

**Inbound close → Inventory receipt:**
- Trigger point: transition receiving → closed
- Action: create receipt per accepted line quantity
- Idempotency: deterministic key per `(tenant_id, shipment_id, line_id, action='receipt')`
- Store returned `receipt_id` on `shipment_lines.inventory_ref_id`

**Outbound ship → Inventory issue:**
- Trigger point: transition packed → shipped
- Action: create issue per shipped line quantity
- Idempotency: deterministic key per `(tenant_id, shipment_id, line_id, action='issue')`
- Store returned `issue_id` similarly

### AP (inbound linkage)

**Purpose**: 3-way match traceability PO ↔ receipt ↔ bill.

- Consumes `ap.purchase_order.approved` to auto-create inbound expected shipment (optional; can be tenant-configured)
- Stores `po_id` / `po_line_id` on inbound lines
- Exposes query endpoints for AP to locate receipts by PO line

### AR / Sales Orders (outbound linkage)

**Purpose**: Fulfillment traceability and customer shipment status.

- Consumes `ar.sales_order.released` (or equivalent) to auto-create outbound shipment
- Stores `source_ref_type` / `source_ref_id` on outbound lines
- Exposes query endpoint for upstream modules/UI: shipment lookup by sales order ref

### Party Master (carrier identity)

**Purpose**: One canonical identity system for external orgs.

- Stores `carrier_party_id` on shipments
- No duplication of carrier data inside shipping-receiving

### GL (future, optional)

**Purpose**: Post freight charges / landed cost and accruals.

- v0.1.0: freight cost captured on shipment (optional field)
- v1.0.0: freight audit + landed cost allocation triggers GL postings

---

## 7. v0.1.0 Scope (current delivery)

10 beads (7 base + 3 gap-fillers):

1. **Scaffold** — Axum service, JWT, health, outbox, metrics, RBAC, db skeleton *(DONE)*
2. **DB schema** — shipments (direction), lines, refs, indexes, outbox/idempotency *(DONE)*
3. **Domain model** — inbound+outbound state machines, guards, invariants, repo layer, event contracts *(IN PROGRESS)*
4. **HTTP API** — inbound/outbound endpoints, tenant-scoped, RequirePermissionsLayer, metrics instrumentation
5. **Event consumers** — PO approved, SO released, inventory receipt confirmed; idempotent; lag metrics
6. **Inventory integration** — inbound close → receipts; outbound ship → issues; exactly-once idempotent keys
7. **AP/AR linkage** — PO refs inbound, SO/source refs outbound, query endpoints, ref indexes
8. **Read models** — status history + dashboard aggregation endpoints
9. **Integrated tests** — both directions, tenant isolation, idempotency, invariants, consumers, RBAC, outbox, dashboards

---

## 8. v0.2.0 Scope (next)

Focus: expand usability without over-engineering.

- **Carrier enhancements** (still Party-owned): preferred carriers per tenant (reference list), carrier service-level selections (ground/2-day/etc.) as simple enums
- **Rate shopping** (lightweight): store quoted freight cost options (no label purchasing yet)
- **ASN support** (inbound): optional advanced shipment notice as pre-receipt expectation
- **Packing slips / basic documents**: generate printable packing slip PDFs (simple template)
- **Returns / RMA** (baseline): outbound return shipment direction variant or separate return workflow
- **Audit & compliance enhancements**: more detailed status history metadata (who/when/why)

---

## 9. v1.0.0 Scope (proven module)

Criteria: production-ready logistics with accounting-grade traceability and operational scale.

**Core additions:**
- Landed cost allocation: allocate freight to receipts (by weight/value/qty); GL posting integration for freight accrual and landed cost capitalization
- Customs / international shipping hooks (data fields + documents)
- Multi-leg shipments (handoffs, partial deliveries)
- Drop-ship and cross-dock workflows
- Barcode/scanning workflows (warehouse execution support)
- Advanced dashboards + reporting exports
- Freight audit workflow (invoice vs quoted freight vs actual)

**Operational hardening:**
- Robust consumer backpressure behavior
- Replay tests and deterministic reprocessing
- High-volume performance baselines and SLO dashboards

---

## 10. Decision Log

| # | Decision | Rationale |
|---|---|---|
| 1 | One module, not two | Shipping and receiving share carrier/tracking logic and unified governance. Direction enum on Shipment. |
| 2 | No ASN in v0.1.0 | PO → receipt path covers MVP; ASN deferred to v0.2.0. |
| 3 | Carrier identity belongs to Party Master | Store `carrier_party_id` only; do not replicate carrier records. |
| 4 | Inventory is the stock ledger of record | This module does not maintain its own stock; it triggers inventory receipts/issues at lifecycle boundaries. |
| 5 | Guard → Mutation → Outbox is mandatory | All lifecycle mutations and emitted events are in a single DB transaction with outbox insert. |
| 6 | Tenant isolation is mandatory | `tenant_id` on every table and query; tenant derived from VerifiedClaims, never from request body. |
| 7 | RBAC required for mutation | All POST/PUT/PATCH/DELETE gated behind RequirePermissionsLayer. |
| 8 | Observability from day 1 | Metrics endpoint, request duration histogram, request counters, consumer lag gauges, dashboard endpoints. |
| 9 | No freight/GL integration in v0.1.0 | Optional freight cost field stored; actual GL posting deferred to v1.0.0. |
