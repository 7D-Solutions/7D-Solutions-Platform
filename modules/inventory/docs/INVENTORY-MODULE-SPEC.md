# Inventory Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.0)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | CopperRiver | Initial vision doc — documented from source code, migrations, event contracts, and integration tests |
| 2.0 | 2026-02-24 | CopperRiver | Fresh-eyes review: fixed 9 inaccuracies — table names (valuation, low_stock_state), missing columns (reorder_policies.safety_stock/max_qty), invented columns (cycle_count_lines.variance_qty/adjustment_id), test count, missing events |

---

## The Business Problem

Every business that sells, distributes, or consumes physical goods has the same core problem: **knowing what you have, where it is, what it costs, and what needs reordering.**

Small and mid-size operations track inventory in spreadsheets or their heads. Stock gets oversold because nobody checked the warehouse. COGS is guessed at tax time because nobody tracked which cost layers were consumed. Cycle counts happen annually — if at all — and the gap between book inventory and physical reality grows wider every month. When stock runs low, nobody notices until a customer order can't be filled.

Larger operations buy ERP systems that bundle inventory with everything else, creating vendor lock-in and forcing businesses to adopt modules they don't need. The inventory module in these systems is tied to a specific chart of accounts, a specific purchasing workflow, and a specific set of assumptions about how the business operates.

---

## What the Module Does

The Inventory module is the **authoritative system for stock quantities, cost layers, and warehouse movements** across any type of stockable item. It is **industry-agnostic**: the same data model handles a wholesale distributor, a manufacturer's raw materials warehouse, a retail stockroom, or a service company's parts inventory.

It answers six questions:
1. **What do we have?** — An item master with SKU, unit of measure, GL account references, and tracking mode (none, lot, or serial).
2. **Where is it?** — Multi-warehouse with optional sub-warehouse locations (bins, shelves, zones).
3. **How much did it cost?** — FIFO cost layers tracked per receipt, consumed deterministically on issue.
4. **What's available?** — Real-time on-hand projections with reservation holds, status buckets (available/quarantine/damaged), and quantity-available calculations.
5. **What's running low?** — Reorder policies with configurable reorder points that emit low-stock signals when thresholds are crossed.
6. **What happened?** — An immutable movement ledger recording every receipt, issue, transfer, adjustment, and status change with full traceability.

---

## Who Uses This

The module is a platform service consumed by any vertical application that manages physical stock. It does not have its own frontend — it exposes an API that frontends consume.

### Warehouse Manager / Inventory Controller
- Creates items with SKU, UoM, and GL account mappings
- Defines warehouse locations (bins, shelves, zones)
- Manages reorder policies (reorder point per item/warehouse/location)
- Initiates cycle count tasks and approves results
- Reviews movement history and valuation snapshots
- Performs stock adjustments with documented reasons

### Fulfillment / Operations
- Receives stock (purchase receipts) with cost and lot/serial tracking
- Issues stock against source documents (sales orders, work orders)
- Reserves stock for future fulfillment (with optional TTL)
- Transfers stock between warehouses
- Moves stock between status buckets (available/quarantine/damaged)

### Finance / Accounting
- Relies on FIFO cost layer data for COGS calculations
- Consumes `inventory.item_issued` events for GL journal entries
- Uses valuation snapshots for period-end reporting
- Accesses GL account references (inventory asset, COGS, variance) per item

### System (Event Consumers)
- GL module subscribes to `inventory.item_issued` for COGS posting
- Notifications module subscribes to `inventory.low_stock_triggered` for reorder alerts
- Downstream modules consume transfer, adjustment, and cycle count events

---

## Design Principles

### FIFO Cost Layers Are the Source of Truth
Every stock receipt creates a cost layer. Every issue consumes layers oldest-first, deterministically. The FIFO algorithm is a pure function — no database access, no configuration, no exceptions. The caller locks layers, the algorithm returns consumed slices, and the caller persists. Cost data is never approximated or averaged (except for transfer destination layers, which use weighted-average of consumed source layers).

### Guard → Mutation → Outbox Atomicity
Every state-changing operation follows the same three-step pattern within a single database transaction: (1) validate preconditions (guards), (2) write the mutation (ledger, layers, projections), (3) enqueue the event in the outbox. If any step fails, the entire transaction rolls back. No event without a mutation. No mutation without a guard.

### Immutable Ledger, Mutable Projections
The `inventory_ledger` is append-only — corrections are new entries (adjustments), not edits. On-hand projections (`item_on_hand`, `item_on_hand_by_status`) are materialized caches that can be rebuilt from the ledger. This separation means the audit trail is permanent while the read model is fast.

### Multi-Tracking Modes (None / Lot / Serial)
Each item chooses its tracking mode at creation — it's immutable thereafter. None-tracked items flow freely. Lot-tracked items require a `lot_code` on every receipt and issue, with FIFO restricted to layers within the specified lot. Serial-tracked items require individual serial codes, with quantity derived from the code count. The tracking mode changes the receipt/issue path but not the FIFO engine or ledger structure.

### Standalone First, Integrate Later
Every integration is optional. The module boots and runs without GL, Notifications, AR, or any other module. GL account references on items are opaque strings — Inventory never validates them against GL's chart of accounts. Reorder signals are best-effort. COGS events carry all the data GL needs, but Inventory never calls GL. This is not degraded mode — it is the baseline.

### Idempotency Everywhere
Every mutating endpoint requires a caller-supplied `idempotency_key`. The key is stored with a hash of the request body and the serialized response. Replayed requests return the stored result. Conflicting requests (same key, different body) are rejected. Keys expire after 7 days.

---

## MVP Scope (v0.1.0)

### In Scope (Built)
- Item master CRUD with SKU uniqueness per tenant, GL account refs, UoM, tracking mode
- Unit of measure catalog with item-level conversion tables
- Multi-warehouse support with optional sub-warehouse locations (bins, shelves, zones)
- Stock receipts with FIFO cost layer creation, lot/serial tracking, UoM conversion
- Stock issues with deterministic FIFO consumption, lot-restricted and serial-restricted paths
- Stock reservations (reserve/release/fulfill) with compensating entry model
- Inter-warehouse transfers with atomic dual-leg (FIFO out + cost layer in)
- Stock adjustments (positive/negative) with no-negative guard and allow_negative override
- Status buckets (available/quarantine/damaged) with atomic inter-bucket transfers
- Cycle count tasks: create (snapshot expected), submit (record counts), approve (variance→adjustment)
- Reorder policies per item/warehouse/location with low-stock crossing detection and dedup state
- On-hand projections: quantity_on_hand, quantity_reserved, quantity_available (generated), total_cost_minor
- Status-bucketed projections: item_on_hand_by_status with per-bucket quantity
- Movement history query (full ledger with lot/serial traceability)
- Lot and serial traceability (trace a lot_code or serial_code through movements)
- Valuation snapshots from FIFO layer state as-of a timestamp
- 9 canonical domain events + 3 internal events emitted via outbox
- OpenAPI contract: `contracts/inventory/inventory-v0.1.0.yaml`
- Prometheus metrics: operations counter, HTTP request duration/count, event consumer lag
- Docker image: multi-stage build with cargo-chef caching

### Explicitly Out of Scope for v1
- Purchase order integration (receipts reference POs but don't validate against them)
- Automated reorder (low-stock signal fires but no PO is created)
- Bin-to-bin transfers within a single warehouse (only inter-warehouse transfers)
- Barcode/RFID scanning integration
- Multi-currency cost layers (each layer has a single currency, no FX conversion)
- Inventory forecasting or demand planning
- Kit/BOM assembly (breaking down or assembling composite items)
- Batch processing / bulk import APIs
- Frontend UI (consumed via API by vertical apps or TCP)

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum 0.8 | Port 8092 (default) |
| Database | PostgreSQL | Dedicated database, SQLx 0.8 for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate |
| Auth | JWT via platform `security` crate | Tenant-scoped, role-based; `INVENTORY_MUTATE` permission for writes |
| Projections | Platform `projections` crate | On-hand materialized cache |
| Outbox | Module-owned `inv_outbox` | Standard outbox pattern |
| Idempotency | Module-owned `inv_idempotency_keys` | 7-day TTL, request hash conflict detection |
| Metrics | Prometheus | `/metrics` endpoint with SLO-oriented histograms |
| Crate | `inventory-rs` | Single crate, modular internal layout |
| Docker | Multi-stage with cargo-chef | `Dockerfile.workspace` at module root |

---

## Structural Decisions (The "Walls")

### 1. FIFO is the only costing method
The module implements First-In-First-Out cost layer accounting. No weighted average, no standard cost, no LIFO. FIFO was chosen because it matches physical reality for most goods (oldest stock ships first), is required by IFRS, and is the simplest to implement correctly. The FIFO engine is a pure function that takes sorted layers and a quantity, making it trivially testable.

### 2. Tracking mode is immutable after item creation
An item's tracking mode (none/lot/serial) is set at creation and never changes. Changing tracking mode after stock movements exist would invalidate historical layer associations and break lot/serial traceability. If a business needs to start tracking serials for an item that was previously untracked, they create a new SKU.

### 3. The ledger is append-only — corrections are new entries
No ledger row is ever updated or deleted. Adjustments create new `adjusted` entries. This means the movement ledger is a permanent audit trail. On-hand projections can be rebuilt from scratch by replaying the ledger. This is non-negotiable for financial compliance.

### 4. Reservations use compensating entries, not status flags
When stock is reserved, a new `active` row is inserted and `quantity_reserved` is incremented. When released, a new `released` row referencing the original is inserted and `quantity_reserved` is decremented. The original row is never mutated. This means the full reservation lifecycle is auditable and idempotent.

### 5. All status buckets are maintained via explicit transfers
Stock doesn't implicitly change status. Moving from available to quarantine requires an explicit `POST /api/inventory/status-transfers` with from_status, to_status, and quantity. Each transfer atomically decrements the source bucket and increments the destination. This prevents status drift and maintains bucket-level auditability.

### 6. GL account references are opaque strings
Items carry `inventory_account_ref`, `cogs_account_ref`, and `variance_account_ref` — but Inventory never validates these against GL. They're passed through in events so GL consumers can route journal entries. This keeps Inventory independent of any specific chart of accounts structure.

### 7. Tenant isolation via tenant_id on every table
Standard platform multi-tenant pattern. Every table has `tenant_id` as a non-nullable field. Every index has `tenant_id` as the leading column. Every query filters by `tenant_id`. No exceptions.

### 8. No mocking in tests
Integration tests hit real Postgres. Tests that mock the database test nothing useful. This is a platform-wide standard. The module has 17 integration test files covering receipts, issues, reservations, transfers, adjustments, cycle counts (create, submit, approve), locations, lot/serial tracking, reorder policies, valuation (snapshot, query), low stock, history, and status transfers.

---

## Domain Authority

Inventory is the **source of truth** for:

| Domain Entity | Inventory Authority |
|---------------|--------------------|
| **Items (SKUs)** | Item master: SKU, name, description, UoM, GL account refs, tracking mode (none/lot/serial), active status. Unique SKU per tenant. |
| **FIFO Cost Layers** | Per-receipt cost layers with quantity_received, quantity_remaining, unit_cost_minor. Consumed oldest-first. |
| **Movement Ledger** | Immutable record of every stock movement: received, issued, transfer_out, transfer_in, adjusted. Each entry carries signed quantity, cost, source event, and references. |
| **On-Hand Projections** | Materialized quantity_on_hand, quantity_reserved, quantity_available (generated column), total_cost_minor per item/warehouse/location. |
| **Status Buckets** | Per-item/warehouse quantity broken down by status (available, quarantine, damaged). |
| **Reservations** | Stock holds with compensating entry lifecycle (active → released or fulfilled). |
| **Lots** | Named lot groupings per item/tenant with optional attributes (JSONB: expiry, supplier batch, etc.). |
| **Serial Instances** | Individual serial-tracked units with status (on_hand, issued) and layer association. |
| **Locations** | Physical or logical bins/shelves/zones within warehouses. |
| **UoM Catalog** | Unit of measure definitions and per-item conversion factors. |
| **Reorder Policies** | Reorder point thresholds per item/warehouse/location with dedup state for crossing detection. |
| **Cycle Count Tasks** | Physical count tasks with expected/counted/variance per line, submit/approve lifecycle. |
| **Valuation Snapshots** | Point-in-time FIFO valuation captures per warehouse with per-item breakdown. |
| **Transfers** | Inter-warehouse transfer records linking issue and receipt ledger legs. |
| **Adjustments** | Explicit stock corrections with reason codes, linking to ledger entries. |

Inventory is **NOT** authoritative for:
- GL account balances or journal entries (GL module owns this)
- Purchase order lifecycle (AP/Purchasing module would own this)
- Customer orders or fulfillment status (Orders module would own this)
- Asset depreciation or book value (Fixed-Assets module owns this)
- Maintenance work order parts consumption (Maintenance module references inventory via `part_ref`)

---

## Data Ownership

### Tables Owned by Inventory

All tables use `tenant_id` for multi-tenant isolation. Every query **MUST** filter by `tenant_id`.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **items** | Item master (SKU catalog) | `id`, `tenant_id`, `sku`, `name`, `description`, `inventory_account_ref`, `cogs_account_ref`, `variance_account_ref`, `uom`, `base_uom_id`, `tracking_mode` (none\|lot\|serial), `active` |
| **inventory_ledger** | Immutable movement log | `id` (BIGSERIAL), `entry_id` (UUID), `tenant_id`, `item_id`, `warehouse_id`, `location_id`, `entry_type` (received\|issued\|transfer_out\|transfer_in\|adjusted), `quantity` (signed), `unit_cost_minor`, `currency`, `source_event_id`, `source_event_type`, `reference_type`, `reference_id`, `notes`, `posted_at` |
| **inventory_layers** | FIFO cost layers | `id`, `tenant_id`, `item_id`, `warehouse_id`, `ledger_entry_id`, `received_at`, `quantity_received`, `quantity_remaining`, `unit_cost_minor`, `currency`, `lot_id`, `exhausted_at` |
| **layer_consumptions** | FIFO consumption audit trail | `layer_id`, `ledger_entry_id`, `quantity_consumed`, `unit_cost_minor`, `consumed_at` |
| **inventory_reservations** | Stock holds (compensating entry) | `id`, `tenant_id`, `item_id`, `warehouse_id`, `quantity`, `status` (active\|released\|fulfilled), `reverses_reservation_id`, `reference_type`, `reference_id`, `reserved_at`, `released_at`, `expires_at` |
| **item_on_hand** | Materialized on-hand projection | `tenant_id`, `item_id`, `warehouse_id`, `location_id`, `quantity_on_hand`, `available_status_on_hand`, `quantity_reserved`, `quantity_available` (generated), `total_cost_minor`, `currency`, `last_ledger_entry_id`, `projected_at` |
| **item_on_hand_by_status** | Status-bucketed on-hand | `tenant_id`, `item_id`, `warehouse_id`, `status` (available\|quarantine\|damaged), `quantity_on_hand` |
| **uoms** | Unit of measure catalog | `id`, `tenant_id`, `code`, `name` |
| **item_uom_conversions** | Per-item UoM conversion factors | `id`, `tenant_id`, `item_id`, `from_uom_id`, `to_uom_id`, `factor` |
| **inventory_lots** | Named lot groups | `id`, `tenant_id`, `item_id`, `lot_code`, `attributes` (JSONB — expiry, supplier batch, etc.) |
| **inventory_serial_instances** | Individual serial-tracked units | `id`, `tenant_id`, `item_id`, `serial_code`, `status` (on_hand\|issued), `ledger_entry_id`, `layer_id` |
| **locations** | Warehouse sub-locations (bins/shelves) | `id`, `tenant_id`, `warehouse_id`, `code`, `name`, `description`, `is_active` |
| **inv_status_transfers** | Status bucket transfer log | `id`, `tenant_id`, `item_id`, `warehouse_id`, `from_status`, `to_status`, `quantity`, `event_id`, `transferred_at` |
| **inv_transfers** | Inter-warehouse transfer records | `id`, `tenant_id`, `item_id`, `from_warehouse_id`, `to_warehouse_id`, `quantity`, `event_id`, `issue_ledger_id`, `receipt_ledger_id`, `transferred_at` |
| **inv_adjustments** | Stock adjustment records | `id`, `tenant_id`, `item_id`, `warehouse_id`, `location_id`, `quantity_delta`, `reason`, `event_id`, `ledger_entry_id`, `adjusted_at` |
| **cycle_count_tasks** | Cycle count task headers | `id`, `tenant_id`, `warehouse_id`, `location_id`, `status` (open\|submitted\|approved), timestamps |
| **cycle_count_lines** | Per-item count lines within a task | `id`, `task_id`, `tenant_id`, `item_id`, `expected_qty`, `counted_qty` (NULL until submitted). Variance and adjustment_id are computed at approve time, not stored. |
| **reorder_policies** | Reorder point thresholds | `id`, `tenant_id`, `item_id`, `location_id` (nullable — NULL = global), `reorder_point`, `safety_stock`, `max_qty` (nullable), `notes`, `created_by`, `updated_by` |
| **inv_low_stock_state** | Dedup state for low-stock signals | `id`, `tenant_id`, `item_id`, `location_id` (nullable — matches reorder_policies), `below_threshold` (bool), `updated_at` |
| **inventory_valuation_snapshots** | Point-in-time valuation headers | `id`, `tenant_id`, `warehouse_id`, `location_id` (nullable), `as_of`, `total_value_minor`, `currency` |
| **inventory_valuation_lines** | Per-item valuation detail | `id`, `snapshot_id`, `item_id`, `warehouse_id`, `location_id` (nullable), `quantity_on_hand`, `unit_cost_minor`, `total_value_minor`, `currency` |
| **inv_outbox** | Event outbox | Standard outbox schema |
| **inv_processed_events** | Event consumer deduplication | `id`, `event_id`, `event_type`, `processor`, `processed_at` |
| **inv_idempotency_keys** | Request deduplication | `tenant_id`, `idempotency_key`, `request_hash`, `response_body`, `status_code`, `expires_at` |

**Monetary Precision:** All monetary amounts use **integer minor units** (e.g., `unit_cost_minor` in cents). Currency stored as 3-letter ISO 4217 code.

**Tenant Isolation:** Every table includes `tenant_id` as a non-nullable field. All indexes include `tenant_id` as the leading column.

### Data NOT Owned by Inventory

Inventory **MUST NOT** store:
- GL journal entries, account balances, or chart of accounts
- Purchase order headers/lines or vendor data
- Customer order headers/lines or customer data
- Asset depreciation schedules or book values
- Maintenance work order or plan data (Maintenance references inventory via `part_ref`)
- Warehouse physical addresses or operating hours (warehouses are opaque UUIDs in v1)

---

## Events Produced

All events use the platform `EventEnvelope` and are written to the module outbox atomically with the triggering mutation. Schema version: `1.0.0`.

| Event | Trigger | Key Payload Fields |
|-------|---------|-------------------|
| `inventory.item_received` | Stock receipt posted | `receipt_line_id`, `item_id`, `sku`, `warehouse_id`, `quantity`, `unit_cost_minor`, `currency`, `purchase_order_id` |
| `inventory.item_issued` | Stock issue posted (FIFO consumed) | `issue_line_id`, `item_id`, `sku`, `warehouse_id`, `quantity`, `total_cost_minor`, `currency`, `consumed_layers[]`, `source_ref` |
| `inventory.adjusted` | Manual stock adjustment | `adjustment_id`, `item_id`, `sku`, `warehouse_id`, `quantity_delta`, `reason` |
| `inventory.transfer_completed` | Both legs of inter-warehouse transfer posted | `transfer_id`, `item_id`, `sku`, `from_warehouse_id`, `to_warehouse_id`, `quantity` |
| `inventory.status_changed` | Status bucket transfer | `transfer_id`, `item_id`, `sku`, `warehouse_id`, `from_status`, `to_status`, `quantity` |
| `inventory.cycle_count_submitted` | Cycle count task submitted | `task_id`, `warehouse_id`, `location_id`, `lines[]` (expected, counted, variance per item) |
| `inventory.cycle_count_approved` | Cycle count approved, adjustments posted | `task_id`, `warehouse_id`, `location_id`, `lines[]` (variance + adjustment_id per item) |
| `inventory.low_stock_triggered` | Available qty crosses below reorder point | `item_id`, `warehouse_id`, `location_id`, `reorder_point`, `available_qty` |
| `inventory.valuation_snapshot_created` | Valuation snapshot built from FIFO state | `snapshot_id`, `warehouse_id`, `as_of`, `total_value_minor`, `lines[]` (per-item valuation) |

Internal events (not in canonical contract, used for outbox tracking):
- `inventory.item_reserved` — stock reservation created
- `inventory.reservation_released` — reservation compensating entry posted
- `inventory.reservation_fulfilled` — reservation fulfilled (stock physically deducted)

---

## Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| *(None in v1)* | — | Inventory is event-producing only in v1. Future: consume purchase order events to auto-create receipt drafts. |

---

## Integration Points

### GL (Event-Driven, One-Way)

`inventory.item_issued` carries `total_cost_minor`, `consumed_layers[]` (with per-layer unit cost and extended cost), `currency`, and GL account refs are available on the item. A GL consumer subscribes and posts COGS journal entries. **Inventory never calls GL.** GL subscribes to the event.

### Maintenance (Optional, Read-Only Reference)

Maintenance work order parts can reference an inventory SKU via `part_ref` (opaque UUID). In v1, this is informational only. Future beads may enable active integration (HTTP commands to reserve/issue parts from inventory when a work order is completed).

### Notifications (Event-Driven, One-Way)

The Notifications module can subscribe to:
- `inventory.low_stock_triggered` → sends reorder alerts
- `inventory.cycle_count_approved` → sends audit notifications

**Inventory never calls Notifications.** Notifications subscribes to the events.

### AR (Future, Not in v1)

For businesses that bill customers for stock issued, a future integration would allow issued events to generate AR invoice line items. Not designed or implemented in v1.

### AP / Purchasing (Future, Not in v1)

Receipts optionally carry `purchase_order_id` for reference, but there is no active validation against a purchase order module. Future integration would link receipts to PO lines for three-way matching.

---

## Invariants

1. **Tenant isolation is unbreakable.** Every query filters by `tenant_id`. No cross-tenant data leakage.
2. **FIFO consumption is deterministic.** Oldest layer first, tie-break by `ledger_entry_id`. The algorithm is a pure function with no configuration.
3. **Outbox atomicity.** Every state-changing mutation writes its event to the outbox in the same database transaction. No silent event loss.
4. **Ledger immutability.** No ledger row is ever updated or deleted. Corrections create new entries.
5. **Available = On Hand - Reserved.** `quantity_available` is a generated column. No manual update.
6. **Tracking mode immutability.** An item's tracking mode (none/lot/serial) cannot change after creation.
7. **SKU uniqueness per tenant.** Enforced by both application guard and database unique constraint.
8. **Idempotency key uniqueness per tenant.** Same key + same body = replayed response. Same key + different body = 409 conflict.
9. **FIFO layer quantity_remaining never goes negative.** Enforced by availability check before consumption.
10. **Status bucket transfers are zero-sum.** Source bucket decremented by exactly the amount destination is incremented.
11. **Serial codes are unique per tenant+item.** Enforced by database unique constraint.
12. **Cost layer extended_cost = quantity x unit_cost.** Precomputed, no floating point.
13. **No forced dependencies.** The module boots and functions without GL, Notifications, Maintenance, or any other module running.

---

## API Surface (Summary)

Full OpenAPI contract: `contracts/inventory/inventory-v0.1.0.yaml`

### Items
- `POST /api/inventory/items` — Create item (SKU, UoM, GL refs, tracking mode)
- `GET /api/inventory/items/{id}` — Get item detail
- `PUT /api/inventory/items/{id}` — Update mutable fields (name, description, GL refs, UoM)
- `POST /api/inventory/items/{id}/deactivate` — Soft-delete item

### Units of Measure
- `POST /api/inventory/uoms` — Create UoM definition
- `GET /api/inventory/uoms` — List UoMs
- `POST /api/inventory/items/{id}/uom-conversions` — Create item-level conversion
- `GET /api/inventory/items/{id}/uom-conversions` — List conversions for item

### Stock Movements
- `POST /api/inventory/receipts` — Receive stock (creates FIFO layer)
- `POST /api/inventory/issues` — Issue stock (consumes FIFO layers)
- `POST /api/inventory/transfers` — Inter-warehouse transfer (atomic dual-leg)
- `POST /api/inventory/adjustments` — Manual stock adjustment

### Reservations
- `POST /api/inventory/reservations/reserve` — Create stock hold
- `POST /api/inventory/reservations/release` — Release stock hold (compensating entry)
- `POST /api/inventory/reservations/{id}/fulfill` — Fulfill reservation

### Status Management
- `POST /api/inventory/status-transfers` — Move qty between status buckets

### Cycle Counts
- `POST /api/inventory/cycle-count-tasks` — Create cycle count task (snapshots expected qty)
- `POST /api/inventory/cycle-count-tasks/{task_id}/submit` — Submit counted quantities
- `POST /api/inventory/cycle-count-tasks/{task_id}/approve` — Approve (variances become adjustments)

### Reorder Policies
- `POST /api/inventory/reorder-policies` — Create reorder policy
- `PUT /api/inventory/reorder-policies/{id}` — Update reorder policy
- `GET /api/inventory/reorder-policies/{id}` — Get reorder policy
- `GET /api/inventory/items/{item_id}/reorder-policies` — List policies for item

### Locations
- `POST /api/inventory/locations` — Create location (bin/shelf/zone)
- `GET /api/inventory/locations/{id}` — Get location
- `GET /api/inventory/warehouses/{warehouse_id}/locations` — List locations for warehouse
- `PUT /api/inventory/locations/{id}` — Update location
- `POST /api/inventory/locations/{id}/deactivate` — Soft-delete location

### Queries
- `GET /api/inventory/items/{item_id}/history` — Movement history
- `GET /api/inventory/items/{item_id}/lots` — List lots for item
- `GET /api/inventory/items/{item_id}/serials` — List serial instances for item
- `GET /api/inventory/items/{item_id}/lots/{lot_code}/trace` — Trace lot through movements
- `GET /api/inventory/items/{item_id}/serials/{serial_code}/trace` — Trace serial through movements
- `POST /api/inventory/valuation-snapshots` — Create valuation snapshot
- `GET /api/inventory/valuation-snapshots` — List snapshots
- `GET /api/inventory/valuation-snapshots/{id}` — Get snapshot detail

### Operational
- `GET /api/health` — Health check
- `GET /api/ready` — Readiness check
- `GET /api/version` — Version info
- `GET /healthz` — Kubernetes liveness
- `GET /metrics` — Prometheus metrics

---

## Decision Log

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-18 | FIFO is the only costing method | Matches physical reality for most goods, required by IFRS, simplest to implement correctly as a pure function | Platform Orchestrator |
| 2026-02-18 | Tracking mode (none/lot/serial) is immutable after creation | Changing mode after stock movements exist would invalidate historical layer associations and break lot/serial traceability | Platform Orchestrator |
| 2026-02-18 | Ledger is append-only, corrections are new entries | Permanent audit trail for financial compliance; on-hand projections can be rebuilt from ledger replay | Platform Orchestrator |
| 2026-02-18 | Reservations use compensating entries, not status flags | Full reservation lifecycle is auditable and idempotent; original rows never mutated | Platform Orchestrator |
| 2026-02-18 | GL account references are opaque strings, not validated | Keeps Inventory independent of any specific chart of accounts; GL consumers interpret the refs | Platform Orchestrator |
| 2026-02-18 | Every mutating endpoint requires idempotency key | Prevents double-processing on network retries; 7-day TTL with request hash conflict detection | Platform Orchestrator |
| 2026-02-18 | Status buckets (available/quarantine/damaged) require explicit transfers | Prevents status drift; maintains bucket-level auditability; zero-sum transfers only | Platform Orchestrator |
| 2026-02-18 | FIFO algorithm is a pure function (no DB, no config) | Trivially testable; caller handles locking and persistence; single responsibility | Platform Orchestrator |
| 2026-02-18 | Locations are optional in v1 (all flows work with location_id = NULL) | Simpler initial deployment; location-aware flows activated by providing location_id | Platform Orchestrator |
| 2026-02-18 | Low-stock signal uses crossing detection with dedup state | Prevents alert spam; signal fires once per crossing, re-arms when stock rises above threshold | Platform Orchestrator |
| 2026-02-18 | Cycle counts use snapshot/submit/approve lifecycle | Expected qty captured at creation (no drift during counting); approval is the boundary where variances become adjustments | Platform Orchestrator |
| 2026-02-18 | Weighted-average unit cost for transfer destination layers | Source layers consumed via FIFO may have different costs; destination gets a single layer with blended cost | Platform Orchestrator |
| 2026-02-18 | No mocking in tests — integrated tests against real Postgres | Platform-wide standard; mocked tests provide false confidence; 17 integration test files | Platform Orchestrator |
| 2026-02-18 | Tenant isolation via tenant_id on every table | Standard platform multi-tenant pattern; all indexes include tenant_id as leading column | Platform Orchestrator |

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`
