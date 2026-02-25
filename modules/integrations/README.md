# Integrations Module

Inbound webhook gateway and external system connector. Receives webhooks from third-party systems (Stripe, GitHub, etc.), verifies signatures, routes events to domain subjects via the outbox, and manages external reference mappings.

## Architecture

- **Language**: Rust
- **Framework**: Axum
- **Database**: PostgreSQL (port 5449)
- **Event Bus**: NATS
- **Port**: 8099 (default)

## Key Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/webhooks/inbound/{system}` | Receive inbound webhook |
| CRUD | `/api/integrations/external-refs` | External reference mappings |
| GET | `/api/integrations/external-refs/by-entity` | Look up refs by internal entity |
| GET | `/api/integrations/external-refs/by-system` | Look up refs by external system |
| POST | `/api/integrations/connectors` | Register a connector |
| POST | `/api/integrations/connectors/{id}/test` | Test connector connectivity |
| GET | `/api/integrations/connectors/types` | List available connector types |
| GET | `/api/integrations/connectors` | List registered connectors |

## Database Tables

- `integrations_webhook_endpoints` — registered webhook endpoint configurations
- `integrations_webhook_ingest` — raw inbound webhook log
- `integrations_external_refs` — entity-to-external-system reference mappings
- `integrations_connector_configs` — connector configurations
- `integrations_outbox` — outbound domain events
- `integrations_processed_events` — deduplication tracking
- `integrations_idempotency_keys` — idempotency enforcement

## Events

**Emitted** (via outbox):
- `payment.received` — Stripe payment_intent.succeeded
- `payment.failed` — Stripe payment_intent.payment_failed
- `invoice.paid.external` — Stripe invoice.payment_succeeded
- `subscription.created.external` — Stripe subscription created
- `subscription.cancelled.external` — Stripe subscription deleted
- `repository.push` / `repository.pull_request` — GitHub events

## Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | Yes | — | PostgreSQL connection string |
| `BUS_TYPE` | No | `inmemory` | Event bus: `inmemory` or `nats` |
| `NATS_URL` | No | `nats://localhost:4222` | NATS server URL |
| `HOST` | No | `0.0.0.0` | Bind address |
| `PORT` | No | `8099` | HTTP port |
| `ENV` | No | `development` | Environment name |
| `CORS_ORIGINS` | No | — | Comma-separated allowed origins |
| `STRIPE_WEBHOOK_SECRET` | No | — | Stripe signature verification secret |
| `GITHUB_WEBHOOK_SECRET` | No | — | GitHub signature verification secret |

## Documentation

- **[INTEGRATIONS-MODULE-SPEC.md](./docs/INTEGRATIONS-MODULE-SPEC.md)**: Full specification

## Development

```bash
./scripts/cargo-slot.sh build -p integrations-rs
./scripts/cargo-slot.sh test -p integrations-rs
```
