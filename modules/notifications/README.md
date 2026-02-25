# Notifications Module

Listens for domain events from other modules and creates scheduled notifications for delivery. Handles dispatch lifecycle including retries and orphan recovery.

## Architecture

- **Language**: Rust
- **Framework**: Axum
- **Database**: PostgreSQL
- **Event Bus**: NATS (outbox pattern + consumer)
- **Default Port**: 8089

## Key Endpoints

### Ops
- `GET /api/health` — liveness probe
- `GET /api/ready` — readiness probe (DB check)
- `GET /api/version` — module version and schema version

Admin routes are available under the admin router (requires `notifications.mutate` permission).

## Database Tables

- `scheduled_notifications` — notification queue (pending, claimed, sent, failed)
- `events_outbox` — transactional outbox for domain events
- `processed_events` — idempotent inbound event deduplication
- `failed_events` — dead-letter queue for unprocessable events

## Events

### Consumed
- `ar.events.invoice.issued` — creates notification when invoice is issued
- `payments.events.payment.succeeded` — creates notification on successful payment
- `payments.events.payment.failed` — creates notification on failed payment

### Background Tasks
- **Dispatch loop**: polls `scheduled_notifications` on a configurable interval, claims pending notifications, and dispatches them via the configured sender
- **Orphan recovery**: on startup, resets notifications stuck in `claimed` status (e.g., from a crash)

## Configuration

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | _(required)_ | PostgreSQL connection string |
| `BUS_TYPE` | `inmemory` | Event bus: `inmemory` or `nats` |
| `NATS_URL` | `nats://localhost:4222` | NATS server URL |
| `HOST` | `0.0.0.0` | Bind address |
| `PORT` | `8089` | HTTP port |
| `CORS_ORIGINS` | `*` | Comma-separated allowed origins |
| `NOTIFICATIONS_DISPATCH_INTERVAL_SECS` | `60` | Seconds between dispatch polls |
