# Inventory Module

Manages item master data, stock movements (receipts, issues, transfers, adjustments), reservations, lot/serial tracking, cycle counts, reorder policies, and inventory valuation.

## Architecture

- **Language**: Rust
- **Framework**: Axum
- **Database**: PostgreSQL
- **Default Port**: 8092

## Key Endpoints

### Item Master
- `POST /api/inventory/items` — create item
- `GET  /api/inventory/items/{id}` — get item
- `PUT  /api/inventory/items/{id}` — update item

### Stock Movements
- `POST /api/inventory/receipts` — receive stock
- `POST /api/inventory/issues` — issue stock
- `POST /api/inventory/transfers` — transfer between locations
- `POST /api/inventory/adjustments` — stock adjustment
- `POST /api/inventory/status-transfers` — change stock status

### Reservations
- `POST /api/inventory/reservations/reserve` — reserve stock
- `POST /api/inventory/reservations/release` — release reservation
- `POST /api/inventory/reservations/{id}/fulfill` — fulfill reservation

### Lot & Serial Tracking
- `GET /api/inventory/items/{item_id}/lots` — lots for item
- `GET /api/inventory/items/{item_id}/serials` — serials for item
- `GET /api/inventory/items/{item_id}/lots/{lot_code}/trace` — lot trace
- `GET /api/inventory/items/{item_id}/serials/{serial_code}/trace` — serial trace
- `GET /api/inventory/items/{item_id}/history` — movement history

### Cycle Counts
- `POST /api/inventory/cycle-count-tasks` — create count task
- `POST /api/inventory/cycle-count-tasks/{task_id}/submit` — submit count
- `POST /api/inventory/cycle-count-tasks/{task_id}/approve` — approve count

### Reorder Policies & Valuation
- `POST /api/inventory/reorder-policies` — create policy
- `GET  /api/inventory/reorder-policies/{id}` — get policy
- `POST /api/inventory/valuation-snapshots` — create snapshot
- `GET  /api/inventory/valuation-snapshots` — list snapshots

### Locations & UoM
- `POST /api/inventory/locations` — create location
- `GET  /api/inventory/locations/{id}` — get location
- `POST /api/inventory/uoms` — create unit of measure
- `GET  /api/inventory/uoms` — list UoMs

### Ops
- `GET /api/health`, `GET /api/ready`, `GET /api/version`

## Database Tables

- `items` — item master (SKU, tracking mode, status)
- `inventory_ledger` — immutable stock movement log
- `fifo_layers` — FIFO cost layers for valuation
- `inventory_reservations` — stock reservations
- `item_on_hand` — materialized on-hand projection
- `inventory_lots` — lot tracking records
- `inventory_serial_instances` — serial number tracking
- `status_buckets` — stock status quantities
- `locations` — warehouse locations
- `status_transfers` — status transfer history
- `adjustments` — stock adjustment records
- `cycle_count_tasks` / `cycle_count_lines` — cycle count workflow
- `inv_transfers` — inter-location transfer records
- `reorder_policies` — min/max reorder rules
- `valuation_snapshots` / `valuation_snapshot_lines` — point-in-time valuation
- `low_stock_state` — low-stock alert tracking
- `events_outbox` / `processed_events` — outbox pattern tables

## Events Emitted

- `inventory.item_received` — stock receipt posted
- `inventory.item_issued` — stock issue posted
- `inventory.adjusted` — stock adjustment applied
- `inventory.transfer_completed` — inter-location transfer done
- `inventory.cycle_count_submitted` — count submitted for review
- `inventory.cycle_count_approved` — count approved and applied
- `inventory.status_changed` — stock status changed
- `inventory.low_stock_triggered` — item fell below reorder point
- `inventory.valuation_snapshot_created` — valuation snapshot taken

## Configuration

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | _(required)_ | PostgreSQL connection string |
| `HOST` | `0.0.0.0` | Bind address |
| `PORT` | `8092` | HTTP port |
| `CORS_ORIGINS` | `*` | Comma-separated allowed origins |
