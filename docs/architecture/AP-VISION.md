# Accounts Payable (AP) Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

The AP module is the **authoritative system for vendor management, purchase orders, vendor bills, and payment execution** on the payables side. AP owns the procure-to-pay lifecycle: PO creation → receipt linking → bill matching (3-way match) → payment run execution. AP emits events at each lifecycle boundary for downstream modules (Shipping-Receiving, Fixed Assets, GL).

### Non-Goals

AP does **NOT**:
- Own inventory stock levels (owned by Inventory)
- Own physical receipt/shipment tracking (owned by Shipping-Receiving)
- Own carrier or party identity (owned by Party Master)
- Own asset capitalization decisions (Fixed Assets consumes AP events)
- Write GL journal entries directly (emits `gl.posting.requested`)

---

## 2. Domain Authority

| Domain Entity | AP Authority |
|---|---|
| **Vendors** | Vendor master data with Party linkage |
| **Purchase Orders** | PO lifecycle (draft → approved → closed), line items |
| **PO Receipt Links** | Links between PO lines and inventory receipts |
| **Vendor Bills** | Supplier invoices for matching and payment |
| **Three-Way Match** | PO ↔ receipt ↔ bill reconciliation |
| **AP Allocations** | Vendor payment scheduling |
| **Payment Runs** | Batch payment execution with line items |
| **AP Tax Snapshots** | Tax calculation snapshots per bill |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `vendors` | Vendor master with party_id linkage |
| `purchase_orders` | PO headers with status lifecycle |
| `po_lines` | Line items per purchase order |
| `po_receipt_links` | Links PO lines to inventory receipt events |
| `vendor_bills` | Supplier invoice records |
| `three_way_match` | Match status between PO, receipt, and bill |
| `ap_allocations` | Payment scheduling records |
| `payment_runs` | Batch payment run headers |
| `payment_run_items` | Per-vendor items in a payment run |
| `payment_run_executions` | Execution status per payment run |
| `ap_tax_snapshots` | Tax calculation snapshots |
| `events_outbox` | Module outbox for NATS |
| `idempotency_keys` | Consumer replay safety |

---

## 4. Events

**Produces:**
- `ap.po_created` — new purchase order created
- `ap.po_approved` — PO approved for fulfillment
- `ap.po_closed` — PO fully received and closed
- `ap.po_line_received_linked` — receipt linked to PO line
- `ap.vendor_created` — new vendor record
- `ap.vendor_updated` — vendor record modified
- `ap.vendor_bill_created` — new vendor bill entered
- `ap.vendor_bill_matched` — bill matched to PO/receipt
- `ap.vendor_bill_approved` — bill approved for payment
- `ap.vendor_bill_voided` — bill voided
- `ap.payment_run_created` — batch payment run initiated
- `ap.payment_executed` — vendor payment executed

**Consumes:**
- `inventory.item_received` — auto-link receipts to PO lines

---

## 5. Key Invariants

1. Three-way match: bill cannot be approved without matching PO and receipt
2. Payment runs are idempotent (deterministic keys per run)
3. Vendor party_id references Party Master (no duplication)
4. Tenant isolation on every table and query
5. All GL impacts via `gl.posting.requested` events

---

## 6. Integration Map

- **Inventory** → emits `inventory.item_received`; AP links to PO lines
- **Shipping-Receiving** → consumes `ap.po_approved` to create inbound shipments
- **Fixed Assets** → consumes `ap.vendor_bill_approved` for capex capitalization
- **Party Master** → vendor identity via `party_id`
- **GL** → AP emits `gl.posting.requested` for bill/payment postings
- **Treasury** → consumes `ap.payment_executed` to record bank transactions
- **Reporting** → consumes bill/payment events for dashboard caching

---

## 7. Roadmap

### v0.1.0 (current)
- Vendor CRUD with Party linkage
- Purchase order lifecycle (create, approve, close)
- Vendor bill entry and 3-way match
- Payment run scheduling and execution
- PO receipt linking (from inventory events)
- Tax snapshot capture
- Event emission for all lifecycle transitions

### v1.0.0 (proven)
- Vendor portal self-service
- Early payment discount tracking
- Multi-currency bill handling with FX
- Approval workflows with configurable thresholds
- AP aging reports and cash flow forecasting
