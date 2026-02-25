# Party Module

Central registry for business parties (companies and individuals) with contact and address management. Provides the shared party identity referenced by AR, AP, and other modules.

## Architecture

- **Language**: Rust
- **Framework**: Axum
- **Database**: PostgreSQL
- **Event Bus**: NATS (outbox pattern)
- **Default Port**: 8098

## Key Endpoints

### Party CRUD
- `POST /api/party/companies` — create company party
- `POST /api/party/individuals` — create individual party
- `GET  /api/party/parties` — list parties
- `GET  /api/party/parties/search` — search parties
- `GET  /api/party/parties/{id}` — get party
- `PUT  /api/party/parties/{id}` — update party
- `POST /api/party/parties/{id}/deactivate` — deactivate party

### Contacts
- `POST /api/party/parties/{party_id}/contacts` — create contact
- `GET  /api/party/parties/{party_id}/contacts` — list contacts
- `GET  /api/party/contacts/{id}` — get contact
- `PUT  /api/party/contacts/{id}` — update contact
- `DELETE /api/party/contacts/{id}` — delete contact

### Addresses
- `POST /api/party/parties/{party_id}/addresses` — create address
- `GET  /api/party/parties/{party_id}/addresses` — list addresses
- `GET  /api/party/addresses/{id}` — get address
- `PUT  /api/party/addresses/{id}` — update address
- `DELETE /api/party/addresses/{id}` — delete address

### Ops
- `GET /api/health`, `GET /api/ready`, `GET /api/version`

## Database Tables

- `parties` — party master (type: company/individual, name, tax_id, status)
- `party_external_refs` — external system references for parties
- `party_contacts` — contact records (email, phone, role)
- `party_addresses` — address records (billing, shipping, etc.)
- `events_outbox` / `processed_events` — outbox pattern tables

## Events Emitted

- `party.created` — new party registered
- `party.updated` — party details changed
- `party.deactivated` — party deactivated

## Configuration

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | _(required)_ | PostgreSQL connection string |
| `BUS_TYPE` | `inmemory` | Event bus: `inmemory` or `nats` |
| `NATS_URL` | `nats://localhost:4222` | NATS server URL |
| `HOST` | `0.0.0.0` | Bind address |
| `PORT` | `8098` | HTTP port |
| `CORS_ORIGINS` | `*` | Comma-separated allowed origins |
