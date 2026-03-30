# BOM Client Migration Guide

Migrate from hand-written fetch calls to the generated `@7d/bom-client`.

## Install

```bash
npm install @7d/bom-client
```

## Setup

```typescript
// Before: hand-written client
const BASE = "http://localhost:8107";
const headers = {
  Authorization: `Bearer ${token}`,
  "Content-Type": "application/json",
};

// After: generated client
import { createBomClient } from "@7d/bom-client";
const client = createBomClient({ baseUrl: "http://localhost:8107", token });
```

## CRUD Operations

### Create BOM

```typescript
// Before
const resp = await fetch(`${BASE}/api/bom`, {
  method: "POST",
  headers,
  body: JSON.stringify({ part_id: partId, description: "Main assembly" }),
});
const bom = await resp.json(); // untyped

// After
const { data: bom, error } = await client.POST("/api/bom", {
  body: { part_id: partId, description: "Main assembly" },
});
// bom is typed as BomHeader | undefined
// error is typed as ApiError | undefined
```

### Get BOM by ID

```typescript
// Before
const resp = await fetch(`${BASE}/api/bom/${bomId}`, { headers });
const bom = await resp.json();

// After
const { data: bom } = await client.GET("/api/bom/{bom_id}", {
  params: { path: { bom_id: bomId } },
});
```

### List BOMs (paginated)

```typescript
// Before
const resp = await fetch(`${BASE}/api/bom?page=1&page_size=20`, { headers });
const body = await resp.json();
const boms: any[] = body.data;
const pagination: any = body.pagination;

// After
const { data } = await client.GET("/api/bom", {
  params: { query: { page: 1, page_size: 20 } },
});
// data.data is BomHeader[]
// data.pagination is { page, page_size, total_items, total_pages }
```

**List response shape** (unchanged from v1):
```json
{
  "data": [ { "id": "...", "part_id": "...", ... } ],
  "pagination": { "page": 1, "page_size": 20, "total_items": 42, "total_pages": 3 }
}
```

## Explosion (Tree Query)

The explosion endpoint returns a **flat list** of rows, each tagged with
`level` and `parent_part_id` so consumers can reconstruct the tree.

```typescript
// Before
const resp = await fetch(
  `${BASE}/api/bom/${bomId}/explosion?max_depth=10`,
  { headers },
);
const rows: any[] = await resp.json();

// After
const { data: rows } = await client.GET("/api/bom/{bom_id}/explosion", {
  params: {
    path: { bom_id: bomId },
    query: { max_depth: 10 },            // optional, defaults to 20
  },
});
// rows is ExplosionRow[]
```

**ExplosionRow shape:**
```typescript
{
  level: number;            // depth in BOM tree (0 = direct child)
  parent_part_id: string;   // UUID of the parent component
  component_item_id: string; // UUID of this component
  quantity: number;
  scrap_factor: number;
  revision_id: string;
  revision_label: string;
  uom?: string | null;
}
```

### Reconstructing a Tree from Flat Rows

```typescript
import type { ExplosionRow } from "@7d/bom-client";

interface BomTreeNode extends ExplosionRow {
  children: BomTreeNode[];
}

function buildTree(rows: ExplosionRow[], rootPartId: string): BomTreeNode[] {
  const byParent = new Map<string, ExplosionRow[]>();
  for (const row of rows) {
    const list = byParent.get(row.parent_part_id) ?? [];
    list.push(row);
    byParent.set(row.parent_part_id, list);
  }
  function expand(parentId: string): BomTreeNode[] {
    return (byParent.get(parentId) ?? []).map((row) => ({
      ...row,
      children: expand(row.component_item_id),
    }));
  }
  return expand(rootPartId);
}
```

## Error Handling

All error responses return `ApiError`:

```typescript
// Before
if (!resp.ok) {
  const err = await resp.json();
  console.error(err.message); // hope it has 'message'
}

// After
const { data, error, response } = await client.POST("/api/bom", {
  body: { part_id: partId },
});
if (error) {
  // error is typed: { error: string, message: string, request_id: string, fields?: FieldError[] }
  console.error(error.message);
  console.error("Request ID:", error.request_id);
}
```

## ECO (Engineering Change Orders)

```typescript
// Create ECO
const { data: eco } = await client.POST("/api/eco", {
  body: { title: "Rev B changes", created_by: "engineer@co.com" },
});

// Lifecycle: submit → approve → apply
await client.POST("/api/eco/{eco_id}/submit", {
  params: { path: { eco_id: eco!.id } },
  body: { actor: "engineer@co.com" },
});
await client.POST("/api/eco/{eco_id}/approve", {
  params: { path: { eco_id: eco!.id } },
  body: { actor: "manager@co.com" },
});
await client.POST("/api/eco/{eco_id}/apply", {
  params: { path: { eco_id: eco!.id } },
  body: { actor: "admin@co.com" },
});
```

## Type Exports

All schema types are re-exported from the package root:

```typescript
import type {
  BomHeader,
  BomLine,
  BomRevision,
  ExplosionRow,
  WhereUsedRow,
  Eco,
  EcoAuditEntry,
  ApiError,
} from "@7d/bom-client";
```
