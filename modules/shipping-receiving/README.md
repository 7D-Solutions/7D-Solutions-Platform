# Shipping & Receiving Module

Manages inbound and outbound shipments with full lifecycle tracking. Automatically creates shipments from approved purchase orders and released sales orders, tracks line-level receiving/shipping quantities, and emits inventory-affecting events on close/ship/deliver.

Carrier adapter setup lives in [docs/guides/carrier-adapters.md](../../docs/guides/carrier-adapters.md).

## Architecture

- **Language**: Rust
- **Framework**: Axum
- **Database**: PostgreSQL (port 5454)
- **Event Bus**: NATS
- **Port**: 8103 (default)

## Key Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/shipping-receiving/shipments` | List shipments |
| GET | `/api/shipping-receiving/shipments/:id` | Get shipment detail |
| POST | `/api/shipping-receiving/shipments` | Create shipment |
| PATCH | `/api/shipping-receiving/shipments/:id/status` | Transition shipment status |
| POST | `/api/shipping-receiving/shipments/:id/lines` | Add line to shipment |
| POST | `/api/shipping-receiving/shipments/:id/lines/:line_id/receive` | Receive a line |
| POST | `/api/shipping-receiving/shipments/:id/lines/:line_id/accept` | Accept a line |
| POST | `/api/shipping-receiving/shipments/:id/lines/:line_id/ship-qty` | Set shipped quantity |
| POST | `/api/shipping-receiving/shipments/:id/ship` | Mark shipment as shipped |
| POST | `/api/shipping-receiving/shipments/:id/deliver` | Mark shipment as delivered |
| POST | `/api/shipping-receiving/shipments/:id/close` | Close shipment |
| GET | `/api/shipping-receiving/po/:po_id/shipments` | Shipments by PO |
| GET | `/api/shipping-receiving/po-lines/:po_line_id/lines` | Lines by PO line |
| GET | `/api/shipping-receiving/source/:ref_type/:ref_id/shipments` | Shipments by source ref |

## Database Tables

- `shipments` — shipment headers (direction, status, tracking, dates)
- `shipment_lines` — individual line items with quantities
- `sr_events_outbox` — outbound domain events
- `sr_processed_events` — inbound event deduplication

## Events

**Consumed:**
- `ap.po.approved` — creates inbound shipment from approved PO
- `sales.so.released` — creates outbound shipment from released SO

**Emitted** (via outbox):
- `shipping.shipment.created` — new shipment created
- `shipping.shipment.status_changed` — status transition
- `shipping.inbound.closed` — inbound shipment closed (triggers inventory receipt)
- `shipping.outbound.shipped` — outbound shipment shipped (triggers inventory adjustment)
- `shipping.outbound.delivered` — outbound shipment delivered

## Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | Yes | — | PostgreSQL connection string |
| `BUS_TYPE` | No | `inmemory` | Event bus: `inmemory` or `nats` |
| `NATS_URL` | No | `nats://localhost:4222` | NATS server URL |
| `HOST` | No | `0.0.0.0` | Bind address |
| `PORT` | No | `8103` | HTTP port |
| `ENV` | No | `development` | Environment name |
| `CORS_ORIGINS` | No | — | Comma-separated allowed origins |
| `INVENTORY_URL` | No | — | Inventory service URL for stock adjustments |

## Development

```bash
./scripts/cargo-slot.sh build -p shipping-receiving-rs
./scripts/cargo-slot.sh test -p shipping-receiving-rs
```
