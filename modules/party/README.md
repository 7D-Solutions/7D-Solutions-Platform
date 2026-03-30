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
- `GET  /api/party/parties` — list parties (paginated)
- `GET  /api/party/parties/search` — search parties (paginated)
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

## Migration Guide: v1.x → v2.0.0

v2.0.0 changes the response shape of list and search endpoints. Single-entity endpoints (get, create, update) are unchanged.

### List/search endpoints now return paginated envelopes

Before (v1.x):
```json
[{ "id": "...", "display_name": "Acme" }, ...]
```

After (v2.0.0):
```json
{
  "data": [{ "id": "...", "display_name": "Acme" }, ...],
  "pagination": { "page": 1, "page_size": 50, "total_items": 120, "total_pages": 3 }
}
```

Affected endpoints:
- `GET /api/party/parties` — accepts `page` and `page_size` query params
- `GET /api/party/parties/search` — pagination derived from existing `limit`/`offset`

### Sub-collection lists return a data wrapper

Before (v1.x):
```json
[{ "id": "...", "first_name": "Alice" }, ...]
```

After (v2.0.0):
```json
{ "data": [{ "id": "...", "first_name": "Alice" }, ...] }
```

Affected endpoints:
- `GET /api/party/parties/{id}/contacts`
- `GET /api/party/parties/{id}/addresses`
- `GET /api/party/parties/{id}/primary-contacts`

### Error responses use ApiError

Before (v1.x):
```json
{ "error": "not_found", "message": "Party ... not found" }
```

After (v2.0.0):
```json
{ "error": "not_found", "message": "Party ... not found", "request_id": "..." }
```

The `error` and `message` fields are unchanged. `request_id` is new (optional, present when tracing context is available).

### Service layer changes

If you call domain service functions directly:
- `list_parties(pool, app_id, include_inactive)` → `list_parties(pool, app_id, include_inactive, page, page_size)` — returns `(Vec<Party>, i64)` instead of `Vec<Party>`
- `search_parties(pool, app_id, query)` — returns `(Vec<Party>, i64)` instead of `Vec<Party>`
