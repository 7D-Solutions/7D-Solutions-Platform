# Sales-Orders Module — Scope, Boundaries, Contracts (draft v0.1)

**7D Solutions Platform**
**Status:** Draft Specification — bd-ixnbs migration
**Date:** 2026-04-16
**Proposed module name:** `sales-orders`
**Source of migration:** Fireproof ERP (`sales_order/` + `blanket_order/`)
**Cross-vertical applicability:** Every vertical with B2B customers — Fireproof, HuberPower, TrashTech, RanchOrbit

---

## 1. Mission & Non-Goals

### Mission
The Sales-Orders module is the **authoritative system for customer order lifecycle prior to invoicing**. It owns the record of what the customer committed to buy, for how much, when they need it, and how releases against blanket commitments are drawn down.

### Non-Goals
Sales-Orders does **NOT**:
- Own invoices or AR ledger (delegated to AR — invoicing happens when an order ships, not when it's booked)
- Own quoting or RFQ (out of scope per user ruling; verticals keep their own quoting if they want, can pass an opaque `external_quote_ref`)
- Own credit checks or customer aging (AR owns those)
- Execute shipments (delegated to Shipping-Receiving — Sales-Orders emits `shipment.requested.v1`, Shipping-Receiving acts)
- Allocate or reserve stock directly (delegated to Inventory — Sales-Orders emits reservation requests; Inventory owns availability)
- Post to GL (no direct GL side effects; revenue is posted by AR on invoice issuance)
- Manage sales pipeline / opportunities (delegated to CRM-Pipeline module)

---

## 2. Domain Authority

Sales-Orders is the **source of truth** for:

| Domain Entity | Authority |
|---------------|-----------|
| **Sales Orders** | Order lifecycle (draft → booked → in_fulfillment → shipped → closed / cancelled), header terms, line items, totals |
| **Sales Order Lines** | Per-line item, quantity, unit price, line total, promised/required dates |
| **Blanket Orders** | Long-term customer commitments with valid_from/until windows, committed value, terms |
| **Blanket Order Lines** | Committed quantity per part, with released-against-commitment tracking |
| **Blanket Order Releases** | Individual draw-downs against a blanket order line — effectively create discrete sales orders against the parent blanket |

Sales-Orders is **NOT** authoritative for:
- Customer party data (Party module owns companies/contacts/addresses)
- Pricing rules or contract pricing (stays in vertical for now; future `pricing` module may emerge)
- Available stock to promise (Inventory owns availability queries)
- Actual shipped quantities (Shipping-Receiving reports; Sales-Orders mirrors for convenience)

---

## 3. Data Ownership

All tables include `tenant_id`, shared-DB model per platform standard. All monetary values use **integer cents** per AR-MODULE-SPEC pattern.

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **sales_orders** | Order header | `id`, `tenant_id`, `order_number`, `status` (canonical: draft/booked/in_fulfillment/shipped/closed/cancelled), `customer_id` (ref → AR customer), `party_id` (ref → Party), `currency`, `subtotal_cents`, `tax_cents`, `total_cents`, `order_date`, `required_date`, `promised_date`, `external_quote_ref`, `blanket_order_id` (nullable — set when this SO is a blanket release), `blanket_release_id` (nullable), `notes`, `created_by`, `created_at`, `updated_at` |
| **sales_order_lines** | Order line items | `id`, `tenant_id`, `sales_order_id`, `line_number`, `item_id` (ref → Inventory item; nullable for ad-hoc lines), `part_number` (denormalized for display), `description`, `uom`, `quantity`, `unit_price_cents`, `line_total_cents`, `required_date`, `promised_date`, `shipped_qty`, `notes` |
| **blanket_orders** | Long-term customer commitments | `id`, `tenant_id`, `blanket_order_number`, `title`, `customer_id`, `party_id`, `status` (canonical: draft/active/expired/cancelled/closed), `currency`, `total_committed_value_cents`, `valid_from`, `valid_until`, `payment_terms`, `delivery_terms`, `incoterms`, `external_quote_ref`, `notes`, `created_by`, `created_at`, `updated_at` |
| **blanket_order_lines** | Committed per-part quantities | `id`, `tenant_id`, `blanket_order_id`, `line_number`, `item_id`, `part_number`, `part_description`, `uom`, `unit_price_cents`, `committed_qty`, `released_qty`, `shipped_qty`, `notes` |
| **blanket_order_releases** | Individual draws against a blanket line | `id`, `tenant_id`, `blanket_order_id`, `blanket_order_line_id`, `release_number`, `status` (canonical: pending/released/shipped/cancelled), `release_qty`, `shipped_qty`, `requested_delivery_date`, `promised_delivery_date`, `actual_ship_date`, `ship_to_address_id` (ref → Party addresses), `shipping_reference`, `sales_order_id` (ref back to the SO this release generated), `notes`, `created_by`, `created_at`, `updated_at` |
| **sales_order_status_labels** | Per-tenant display labels over canonical statuses | `id`, `tenant_id`, `canonical_status`, `display_label`, `description`, `updated_at`, `updated_by` — unique on (`tenant_id`, `canonical_status`) |
| **blanket_order_status_labels** | Same for blanket orders | Same shape |

**Invariant:** `subtotal_cents` = SUM(line_total_cents), `total_cents` = subtotal_cents + tax_cents. Tax calculation deferred to whichever tax engine the vertical uses (platform has `tax-core`; verticals can plug in).

---

## 4. OpenAPI Surface

### 4.1 Sales Order Endpoints
- `POST /api/sales-orders/orders` — Create SO (draft)
- `POST /api/sales-orders/orders/:id/book` — Finalize (draft → booked); emits booking event, triggers reservation requests to Inventory
- `POST /api/sales-orders/orders/:id/cancel` — Cancel
- `GET /api/sales-orders/orders/:id` — Retrieve SO with lines
- `GET /api/sales-orders/orders` — List with filters: customer_id, status, date ranges, blanket_order_id, etc.
- `PUT /api/sales-orders/orders/:id` — Update header (while in `draft`)
- `POST /api/sales-orders/orders/:id/lines` — Add line
- `PUT /api/sales-orders/orders/:id/lines/:line_id` — Update line (while parent is `draft`)
- `DELETE /api/sales-orders/orders/:id/lines/:line_id` — Remove line (while parent is `draft`)

### 4.2 Blanket Order Endpoints
- `POST /api/sales-orders/blankets` — Create blanket order (draft)
- `POST /api/sales-orders/blankets/:id/activate` — draft → active
- `POST /api/sales-orders/blankets/:id/cancel` — Cancel
- `GET /api/sales-orders/blankets/:id` — Retrieve with lines + release summary
- `GET /api/sales-orders/blankets` — List with filters
- `POST /api/sales-orders/blankets/:id/lines` — Add committed line
- `PUT /api/sales-orders/blankets/:id/lines/:line_id` — Update line

### 4.3 Blanket Release Endpoints
- `POST /api/sales-orders/blankets/:id/lines/:line_id/releases` — Create a release (draw down against commitment); generates a child SO
- `POST /api/sales-orders/releases/:id/ship` — Mark shipped
- `POST /api/sales-orders/releases/:id/cancel` — Cancel release (restores committed-qty balance)
- `GET /api/sales-orders/blankets/:id/releases` — List releases for a blanket

### 4.4 Label Endpoints
- `GET /api/sales-orders/status-labels/:scope` — Scope = `order` or `blanket` or `release`
- `PUT /api/sales-orders/status-labels/:scope/:canonical` — Set tenant display label

---

## 5. Events Produced & Consumed

Platform envelope: `event_id`, `occurred_at`, `tenant_id`, `source_module` (= `"sales-orders"`), `source_version`, `correlation_id`, `causation_id`, `payload`.

### 5.1 Events Produced

| Event name | Trigger | Key payload |
|------------|---------|-------------|
| `sales_orders.order.created.v1` | SO created (draft) | `sales_order_id`, `order_number`, `customer_id`, `currency` |
| `sales_orders.order.booked.v1` | SO transitioned draft → booked | `sales_order_id`, `order_number`, `customer_id`, `total_cents`, `lines` (array of item_id/qty/required_date) — downstream modules react |
| `sales_orders.order.cancelled.v1` | SO cancelled | `sales_order_id`, `reason` |
| `sales_orders.order.shipped.v1` | SO marked shipped (from Shipping-Receiving event) | `sales_order_id`, `shipped_at` |
| `sales_orders.order.closed.v1` | SO closed | `sales_order_id`, `closed_at` |
| `sales_orders.blanket.activated.v1` | Blanket order activated | `blanket_order_id`, `blanket_order_number`, `total_committed_value_cents`, `valid_until` |
| `sales_orders.blanket.expired.v1` | Daily sweep: `valid_until < now()` and status still `active` | `blanket_order_id` |
| `sales_orders.release.created.v1` | Blanket release created | `release_id`, `blanket_order_id`, `line_id`, `release_qty`, `sales_order_id` |
| `sales_orders.reservation.requested.v1` | On booking, per line | `sales_order_id`, `line_id`, `item_id`, `quantity`, `required_date` — Inventory consumes |
| `sales_orders.shipment.requested.v1` | Line approaches promised_date or explicit ship call | `sales_order_id`, `line_id`, `item_id`, `quantity`, `ship_to_address_id` — Shipping-Receiving consumes |
| `sales_orders.invoice.requested.v1` | Line shipped | `sales_order_id`, `line_id`, `customer_id`, `amount_cents`, `currency` — AR consumes |

### 5.2 Events Consumed

| Event name | Source | Behavior |
|------------|--------|----------|
| `inventory.reservation.confirmed.v1` | Inventory | Mark SO line as stock-confirmed; allows fulfillment to progress |
| `inventory.reservation.rejected.v1` | Inventory | Flag SO line as stock-short; surface in reporting, notify sales rep |
| `shipping_receiving.shipment.shipped.v1` | Shipping-Receiving | Update SO line `shipped_qty`; when all lines fully shipped, emit `sales_orders.order.shipped.v1` |
| `ar.invoice.issued.v1` | AR | Link AR invoice_id back onto SO (for reporting cross-reference; not authoritative) |

---

## 6. State Machines

### 6.1 Sales Order Lifecycle
```
draft ──> booked ──> in_fulfillment ──> shipped ──> closed
   │         │             │                │
   └──> cancelled      cancelled       cancelled
```
Terminal: `closed`, `cancelled`. Cannot reopen; create a new order.

### 6.2 Blanket Order Lifecycle
```
draft ──> active ──┬──> expired (valid_until passed)
                   ├──> cancelled
                   └──> closed (all commitment consumed or operator closes)
```

### 6.3 Blanket Release Lifecycle
```
pending ──> released ──> shipped
    │           │
    └───── cancelled
```

---

## 7. Security & Tenant Isolation

- Shared DB, row-level isolation by `tenant_id`.
- Role gates: `sales_orders:order:book`, `sales_orders:order:cancel`, `sales_orders:blanket:activate`, `sales_orders:release:create`, `sales_orders:labels:edit`.
- No PCI data; no PII beyond customer references (party_id points at Party module).

---

## 8. Required Invariants

1. **Booking requires at least one line.** Cannot book an empty SO.
2. **Line totals consistent.** `line_total_cents = quantity * unit_price_cents` (monetary integer arithmetic).
3. **Header totals consistent.** `subtotal_cents = SUM(line.line_total_cents)`, `total_cents = subtotal_cents + tax_cents`.
4. **Blanket release quantity bounded.** `SUM(releases.release_qty) + cancelled_qty <= blanket_line.committed_qty`. Over-release is forbidden.
5. **Blanket line `released_qty` = SUM of non-cancelled release.release_qty.** Maintained by triggers or application-level update on release create/cancel.
6. **Blanket line `shipped_qty` = SUM of release.shipped_qty.** Reflects actual shipments.
7. **SO linked to blanket release references the release's blanket_order_id.** Consistency enforced via FK.
8. **Cannot edit lines of a booked SO.** Only draft SOs accept line modifications. To correct a booked SO, cancel and create new.
9. **Canonical status set is platform-owned.** Tenants rename via status_labels; cannot add/remove canonical values.
10. **Events carry canonical statuses only.** Downstream modules match on canonical codes, not tenant display labels.

---

## 9. Cross-module integration notes

- **Inventory:** On booking, Sales-Orders emits `reservation.requested.v1` per line. Inventory owns the availability check and reservation state. Sales-Orders reflects the outcome but doesn't own stock.
- **Shipping-Receiving:** Receives `shipment.requested.v1`, creates shipment documents, emits `shipment.shipped.v1` back. Sales-Orders updates shipped quantities.
- **AR:** On ship, Sales-Orders emits `invoice.requested.v1`. AR creates the invoice. AR issues GL posting events for revenue recognition — Sales-Orders does not touch GL directly.
- **Party:** `customer_id` references AR customer; `party_id` references Party master record. Ship-to addresses reference Party addresses.
- **Numbering:** `order_number`, `blanket_order_number`, `release_number` allocated via the Numbering module for gap-free sequences.

---

## 10. Open questions

- **Tax calculation.** Tax-Core module exists. Should Sales-Orders call tax-core on line create/update, or defer tax computation to AR at invoice time? Probably compute on booking so the customer-facing total is known at commit; re-verify on invoice. Defer decision to first implementation bead.
- **Pricing.** No platform pricing module today. Verticals pass `unit_price_cents` directly. If contract pricing becomes cross-vertical need, extract later.
- **Quote linkage.** `external_quote_ref` is an opaque string; no FK constraint. Fireproof populates with its local quote_id. Other verticals leave empty or use their external CRM's opaque reference.
- **Partial ship handling.** Current design: line's `shipped_qty` accumulates; SO moves to `shipped` status only when all lines fully shipped. Alternative: per-line shipped status. Recommend: full-line accumulation (simpler); revisit if needed.
- **Backorder semantics.** If Inventory says stock-short on a line, does the SO stay in `booked` with flagged line, or transition to a `backorder` state? Recommend: stay in `booked` with line-level `stock_status` column; avoid extra state.

---

## 11. Migration notes (from Fireproof)

- Fireproof's `sales_order/` (~500 LOC header + lines) and `blanket_order/` (~1,000 LOC with releases) consolidate into this platform module.
- Monetary fields convert from `f64` (Fireproof current) to integer `*_cents` (platform standard per AR).
- Blanket line's `part_number` (string) becomes denormalized helper; `item_id` (UUID, ref → Inventory) becomes authoritative.
- Fireproof's `quote_id` (Uuid, ref → Fireproof quoting) maps to `external_quote_ref` (String, opaque) since quoting stays in Fireproof per user ruling.
- Sample data only — drop Fireproof's tables, create fresh schema on platform, Fireproof rewires to typed client (`platform_client_sales_orders::*`).
