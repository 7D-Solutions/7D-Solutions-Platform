# Inventory Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

The Inventory module is the **authoritative stock ledger** for the platform. It tracks item master data, stock movements (receipts, issues, adjustments, transfers), and provides real-time on-hand quantities and valuations. Inventory is the source of truth for "what do we have, where is it, and what is it worth?"

### Non-Goals

Inventory does **NOT**:
- Own physical shipment/carrier tracking (owned by Shipping-Receiving)
- Own purchase orders or vendor management (owned by AP)
- Own sales orders or invoicing (owned by AR)
- Write GL journal entries directly (valuation events consumed by Reporting/GL)

---

## 2. Domain Authority

| Domain Entity | Inventory Authority |
|---|---|
| **Items** | Item master data, tracking mode (none/lot/serial), descriptions, SKUs |
| **Inventory Ledger** | Immutable transaction journal — every stock movement recorded |
| **FIFO Layers** | Cost layer tracking for FIFO valuation method |
| **Reservations** | Stock reservations against sales orders or transfers |
| **On-Hand Projection** | Materialized available quantities per item/location |
| **UOMs** | Unit of measure definitions and conversions |
| **Lots** | Lot/batch tracking with expiration |
| **Serial Instances** | Individual serial number lifecycle tracking |
| **Status Buckets** | Quality/hold status categories (available, quarantine, damaged) |
| **Locations** | Warehouse and bin location hierarchy |
| **Cycle Counts** | Physical count workflow (count → approve → adjust) |
| **Transfers** | Inter-location stock movements |
| **Reorder Policies** | Min/max and reorder point rules |
| **Valuation Snapshots** | Point-in-time inventory valuation for reporting |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `items` | Item master with tracking_mode, tenant_id |
| `inventory_ledger` | Immutable receipt/issue/adjustment/transfer journal |
| `fifo_layers` | Per-item cost layers for FIFO valuation |
| `inventory_reservations` | Stock reservations against orders |
| `item_on_hand_projection` | Materialized on-hand by item/location/status |
| `uoms` | Unit of measure definitions |
| `inventory_lots` | Lot/batch records with expiration dates |
| `inventory_serial_instances` | Individual serial number records |
| `status_buckets` | Quality status definitions |
| `locations` | Warehouse/bin location hierarchy |
| `status_transfers` | Quality status transition records |
| `adjustments` | Inventory count adjustments |
| `cycle_count_*` | Cycle count headers, lines, approvals |
| `inv_transfers` | Inter-location transfer records |
| `reorder_policies` | Min/max reorder rules per item/location |
| `valuation_snapshots` | Point-in-time valuation captures |
| `low_stock_state` | Low stock detection state per item |
| `events_outbox` | Module outbox for NATS |

---

## 4. Events

**Produces:**
- `inventory.item_received` — stock receipt recorded (triggers AP PO linking)
- `inventory.item_issued` — stock issue recorded
- `inventory.adjusted` — inventory adjustment posted
- `inventory.transfer_completed` — inter-location transfer completed
- `inventory.low_stock_triggered` — item fell below reorder point
- `inventory.cycle_count_submitted` — physical count submitted for approval
- `inventory.cycle_count_approved` — count approved, adjustments posted
- `inventory.status_changed` — quality status transition recorded
- `inventory.valuation_snapshot` — valuation snapshot generated

**Consumes:**
- None (Inventory is a source system — other modules call its HTTP API)

---

## 5. Key Invariants

1. Ledger is append-only — no edits to posted transactions
2. On-hand projection must always reconcile with ledger sum
3. FIFO layers must account for all received cost
4. Serial numbers are globally unique per tenant
5. Lot quantities must sum to on-hand for lot-tracked items
6. Tenant isolation on every table and query
7. Guard → Mutation → Outbox for all stock movements

---

## 6. Integration Map

- **AP** → consumes `inventory.item_received` to link receipts to PO lines
- **Shipping-Receiving** → calls Inventory HTTP API to create receipts (inbound close) and issues (outbound ship)
- **Reporting** → consumes `inventory.valuation_snapshot` for dashboard caching
- **GL** → valuation changes flow through Reporting; future direct GL posting for cost-of-goods

---

## 7. Roadmap

### v0.1.0 (current)
- Item master CRUD with lot/serial tracking modes
- Receipt, issue, adjustment, transfer services
- FIFO cost layer tracking
- Reservation management
- Location/warehouse management
- Cycle count workflow
- Reorder point monitoring and low stock alerts
- Valuation snapshot generation
- Status bucket management and quality transitions
- UOM definitions

### v1.0.0 (proven)
- FIFO/LIFO/weighted average valuation method selection
- Multi-warehouse transfer optimization
- Expiration date management and alerts
- Barcode/scanning integration hooks
- ABC analysis classification
- GL cost-of-goods-sold posting integration
- High-volume ledger performance baselines
