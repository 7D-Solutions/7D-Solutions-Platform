# 7D Platform TypeScript Clients

Each module that exposes an HTTP API has a generated TypeScript client package under `clients/{module}/`. These packages are for vertical developers building frontends against the 7D platform.

## Quick start

### Install a module client

The clients are local packages. Add them to your frontend via the file: protocol in package.json, or copy them into your monorepo:

```json
{
  "dependencies": {
    "@7d/inventory-client": "file:../../platform/clients/inventory",
    "@7d/bom-client":       "file:../../platform/clients/bom",
    "@7d/party-client":     "file:../../platform/clients/party"
  }
}
```

Then:
```bash
npm install
```

### Use a client

```typescript
import { createInventoryClient } from "@7d/inventory-client";

const client = createInventoryClient({
  baseUrl: "https://inventory.your-tenant.7d.internal",
  token: await getJwtToken(),           // RS256 JWT from your auth flow
});

// Fully typed — paths, query params, request/response bodies
const { data, error } = await client.GET("/api/items", {
  params: { query: { page: 1, page_size: 20 } },
});
```

All clients are built on [`openapi-fetch`](https://openapi-ts.dev/openapi-fetch/), which provides type-safe fetch wrappers generated from the OpenAPI spec.

## Available clients

| Package | Module | Description |
|---------|--------|-------------|
| `@7d/ap-client` | AP | Accounts Payable |
| `@7d/ar-client` | AR | Accounts Receivable |
| `@7d/bom-client` | BOM | Bill of Materials |
| `@7d/consolidation-client` | Consolidation | Financial consolidation |
| `@7d/customer-portal-client` | Customer Portal | Customer self-service |
| `@7d/fixed-assets-client` | Fixed Assets | Asset management |
| `@7d/gl-client` | GL | General Ledger |
| `@7d/integrations-client` | Integrations | Third-party connectors |
| `@7d/inventory-client` | Inventory | Stock and warehouse management |
| `@7d/maintenance-client` | Maintenance | Maintenance work orders |
| `@7d/notifications-client` | Notifications | In-app notifications |
| `@7d/numbering-client` | Numbering | Document numbering sequences |
| `@7d/party-client` | Party | Customers, vendors, contacts |
| `@7d/payments-client` | Payments | Payment processing |
| `@7d/pdf-editor-client` | PDF Editor | Document editing |
| `@7d/platform-client-doc-mgmt-client` | Doc Mgmt | Document management |
| `@7d/platform-client-tenant-registry-client` | Tenant Registry | Tenant management |
| `@7d/production-client` | Production | Work orders and shop floor |
| `@7d/quality-inspection-client` | Quality Inspection | QC inspections |
| `@7d/reporting-client` | Reporting | Report generation |
| `@7d/shipping-receiving-client` | Shipping & Receiving | Inbound/outbound shipments |
| `@7d/subscriptions-client` | Subscriptions | Subscription billing |
| `@7d/timekeeping-client` | Timekeeping | Time tracking |
| `@7d/treasury-client` | Treasury | Cash and treasury |
| `@7d/ttp-client` | TTP | Trade terms and pricing |
| `@7d/workflow-client` | Workflow | Business process automation |
| `@7d/workforce-competence-client` | Workforce | Skills and competencies |

## Aggregate client

`clients/api/` is a hand-crafted aggregate that re-exports types and factory functions from several module clients into a single import. It currently covers inventory, BOM, and party.

## Authentication

All platform services require a signed RS256 JWT. Your tenant's auth service issues these. Typical payload:

```json
{
  "sub": "<user-uuid>",
  "tenant_id": "<tenant-uuid>",
  "iss": "auth-rs",
  "aud": "7d-platform",
  "roles": ["admin"],
  "perms": ["inventory.read", "inventory.mutate"]
}
```

Pass it as a Bearer token:
```typescript
const client = createInventoryClient({
  baseUrl: "https://inventory.tenant.example.com",
  token: jwt,
});
```

## Regenerating clients

When a module's OpenAPI spec changes, regenerate its client:

```bash
# Regenerate one module
node tools/ts-codegen/ts-codegen.mjs inventory

# Regenerate all modules (first-time or to add new ones)
node tools/ts-codegen/ts-codegen.mjs --all

# Force-regenerate all (including index.ts — used in CI)
node tools/ts-codegen/ts-codegen.mjs --all --regen
```

The generated files are:
- `src/{module}.d.ts` — always overwritten from the OpenAPI spec
- `src/index.ts` — written once; use `--regen` to overwrite

**Never hand-edit generated files.** They will be overwritten on the next codegen run. The source of truth is the OpenAPI spec.

## How codegen works

1. Reads `clients/{module}/openapi.json`
2. Runs [`openapi-typescript`](https://openapi-ts.dev/) to produce `src/{module}.d.ts`
3. Generates `src/index.ts` with typed factory function and schema type re-exports
4. Creates `package.json` and `tsconfig.json` if absent
5. Runs `tsc --noEmit` to verify the types compile

## CI

The `ts-clients-typecheck` job in `.github/workflows/ci.yml` runs on every PR:
- Regenerates all `.d.ts` files from committed `openapi.json` specs
- Runs `tsc --noEmit` for every module client
- Fails the build if any generated `.d.ts` file has drifted from the committed version

To fix a CI failure, run `node tools/ts-codegen/ts-codegen.mjs --all --regen` locally and commit the updated files.
