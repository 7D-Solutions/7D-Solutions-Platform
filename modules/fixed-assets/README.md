# Fixed Assets Module

Manages the lifecycle of capitalized assets including acquisition, depreciation scheduling, depreciation runs, and disposal with GL posting integration.

## Architecture

- **Language**: Rust
- **Framework**: Axum
- **Database**: PostgreSQL
- **Event Bus**: NATS (outbox pattern)
- **Default Port**: 8104

## Key Endpoints

### Assets & Categories
- `POST /api/fixed-assets/categories` — create asset category
- `GET  /api/fixed-assets/categories` — list categories
- `PUT  /api/fixed-assets/categories/{id}` — update category
- `POST /api/fixed-assets/assets` — create asset
- `GET  /api/fixed-assets/assets` — list assets
- `PUT  /api/fixed-assets/assets/{id}` — update asset

### Depreciation
- `POST /api/fixed-assets/depreciation/schedule` — generate depreciation schedule
- `POST /api/fixed-assets/depreciation/runs` — create depreciation run
- `GET  /api/fixed-assets/depreciation/runs` — list runs
- `GET  /api/fixed-assets/depreciation/runs/{id}` — get run

### Disposals
- `POST /api/fixed-assets/disposals` — dispose asset
- `GET  /api/fixed-assets/disposals` — list disposals
- `GET  /api/fixed-assets/disposals/{id}` — get disposal

### Ops
- `GET /api/health` — liveness probe
- `GET /api/ready` — readiness probe (DB check)
- `GET /api/version` — module version and schema version

## Database Tables

- `fa_asset_categories` — asset category definitions
- `fa_assets` — asset master records (cost, method, useful life, NBV)
- `fa_depreciation_schedules` — per-asset depreciation schedule lines
- `fa_depreciation_runs` — batch depreciation run headers
- `fa_disposals` — asset disposal records (sale/write-off)
- `fa_events_outbox` — transactional outbox for domain events
- `fa_processed_events` — idempotent inbound event deduplication
- `fa_ap_capitalizations` — AP bill capitalizations into assets

## Events

### Emitted (via outbox)
- `asset_created` — new asset registered
- `depreciation_run_completed` — batch depreciation run finished
- `asset_disposed` — asset disposed (sale or write-off)

### Consumed
- `ap.events.bill.approved` — capitalizes AP bills into fixed assets

## Configuration

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | _(required)_ | PostgreSQL connection string |
| `BUS_TYPE` | `inmemory` | Event bus: `inmemory` or `nats` |
| `NATS_URL` | `nats://localhost:4222` | NATS server URL |
| `HOST` | `0.0.0.0` | Bind address |
| `PORT` | `8104` | HTTP port |
| `CORS_ORIGINS` | `*` | Comma-separated allowed origins |
