# TTP Module (Tenant Tenancy & Pricing)

Manages tenant service agreements, metered usage tracking, pricing tiers, and billing runs. Generates invoices by calling the AR module and supports one-time charges alongside recurring subscriptions.

## Architecture

- **Language**: Rust
- **Framework**: Axum
- **Database**: PostgreSQL (port 5450)
- **Event Bus**: NATS
- **Port**: 8100 (default)
- **Version**: 2.1.1 (Proven)

## Key Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/ttp/billing-runs` | Create and execute a billing run |
| GET | `/api/ttp/service-agreements` | List service agreements |
| POST | `/api/metering/events` | Ingest metering/usage events |
| GET | `/api/metering/trace` | Trace a metering event |

## Database Tables

- `ttp_customers` — customer records
- `ttp_service_agreements` — service agreement definitions
- `ttp_one_time_charges` — one-time charge line items
- `ttp_billing_runs` — billing run headers
- `ttp_billing_run_items` — individual items in a billing run
- `ttp_metering_events` — raw metering/usage events
- `ttp_metering_pricing` — pricing tiers for metered usage
- `ttp_processed_events` — deduplication tracking

## Events

None currently published (event bus wired but publishing not yet active).

## Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | Yes | — | PostgreSQL connection string |
| `BUS_TYPE` | No | `inmemory` | Event bus: `inmemory` or `nats` |
| `NATS_URL` | No | `nats://localhost:4222` | NATS server URL |
| `HOST` | No | `0.0.0.0` | Bind address |
| `PORT` | No | `8100` | HTTP port |
| `ENV` | No | `development` | Environment name |
| `CORS_ORIGINS` | No | — | Comma-separated allowed origins |
| `TENANT_REGISTRY_URL` | No | `http://localhost:8092` | Tenant registry service URL |
| `AR_BASE_URL` | No | `http://localhost:8086` | AR service URL for invoice creation |

## Documentation

- **[TTP-MODULE-SPEC.md](./docs/TTP-MODULE-SPEC.md)**: Full specification
- **[REVISIONS.md](./REVISIONS.md)**: Revision history

## Development

```bash
./scripts/cargo-slot.sh build -p ttp-rs
./scripts/cargo-slot.sh test -p ttp-rs
```
