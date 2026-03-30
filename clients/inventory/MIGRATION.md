# Inventory Client Migration Guide

## From hand-written fetch to @7d/inventory-client

### Before (hand-written fetch)

```typescript
// Old: hand-written fetch with inline types
interface Item {
  id: string;
  sku: string;
  name: string;
  // ... incomplete, drifts from actual API
}

async function createItem(token: string, body: Record<string, unknown>): Promise<Item> {
  const res = await fetch("http://localhost:8092/api/inventory/items", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function listItems(token: string, limit: number, offset: number) {
  const res = await fetch(
    `http://localhost:8092/api/inventory/items?limit=${limit}&offset=${offset}`,
    { headers: { Authorization: `Bearer ${token}` } },
  );
  return res.json(); // unknown shape — no type safety
}
```

### After (generated client)

```typescript
import { createInventoryClient } from "@7d/inventory-client";

const client = createInventoryClient({
  baseUrl: "http://localhost:8092",
  token: jwt,
});

// Fully typed — request body validated at compile time
const { data: item, response } = await client.POST("/api/inventory/items", {
  body: {
    sku: "WIDGET-001",
    name: "Widget",
    tracking_mode: "lot",
    cogs_account_ref: "5000",
    inventory_account_ref: "1200",
    variance_account_ref: "5010",
    tenant_id: "",
  },
});
// item is typed as Item | undefined
// response.status is typed (201, 409, 422)

// Paginated list — query params are typed
const { data: list } = await client.GET("/api/inventory/items", {
  params: { query: { limit: 10, offset: 0, search: "WIDGET" } },
});
// list.data is Item[], list.pagination is PaginationMeta
```

## Breaking changes (v1 → v2)

### List endpoint response envelope

**Before (v1):**
```json
{ "items": [...], "total": 42, "limit": 50, "offset": 0 }
```

**After (v2):**
```json
{
  "data": [...],
  "pagination": {
    "page": 1,
    "page_size": 50,
    "total_items": 42,
    "total_pages": 1
  }
}
```

All list endpoints now return `PaginatedResponse<T>` with `data` array and `pagination` metadata. The old `items`/`total`/`limit`/`offset` fields no longer exist.

### Error response envelope

**Before (v1):**
```json
{ "error": "Not found" }
```

**After (v2):**
```json
{
  "error": "not_found",
  "message": "Item not found",
  "request_id": "550e8400-e29b-41d4-a716-446655440000",
  "details": null
}
```

All error responses now return `ApiError` with `error`, `message`, `request_id`, and optional `details`. The `request_id` field enables request tracing through support and logs.

### Query parameter names (list items)

The generated client uses `limit`/`offset` (not `page`/`page_size`) for the list items query. The server converts these to the paginated response format internally.

## Regenerating the client

When the Inventory service's OpenAPI spec changes:

```bash
# From a running service
cd clients/inventory
npm run generate

# From the committed spec (offline)
cd modules/inventory
cargo run --bin openapi_dump > ../../clients/inventory/openapi.json
cd ../../clients/inventory
npm run generate:file
```
