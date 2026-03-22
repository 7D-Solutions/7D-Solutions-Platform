# demo-seed

Deterministic demo-data seeder that writes repeatable tenant data through module
HTTP APIs. Same tenant + seed input always produces the same resource set and
SHA256 digest.

## Modules

demo-seed supports 7 modules, executed in dependency order:

| Order | Module       | Creates                                      | Count |
|-------|-------------|----------------------------------------------|-------|
| 1     | `numbering` | Numbering policies (PO, SO, WO, ECO, etc.)   | 8     |
| 2     | `gl`        | GL chart of accounts + FX rates              | 20 + 2 |
| 3     | `party`     | Customers + suppliers (aerospace companies)   | 5 + 5 |
| 4     | `inventory` | UoMs, warehouse locations, items (parts)      | 5 + 7 + 13 |
| 5     | `bom`       | Bills of materials with revisions + lines     | 5     |
| 6     | `production`| Work centers + routing templates with steps   | 6 + 5 |
| 7     | `ar`        | AR customers + invoices (demo billing data)   | configurable |

**Dependency order:** numbering → gl → party → inventory → bom, production → ar

Modules 1–6 are manufacturing-focused. Module 7 (AR) is for billing/invoicing
demo data and has its own seeding via `seed-dev.sh`.

## Usage

```bash
# Full manufacturing seed
cargo run -p demo-seed -- --tenant dev-test-01 --seed 42

# Specific modules only
cargo run -p demo-seed -- --tenant dev-test-01 --seed 42 --modules numbering,gl

# All modules including AR
cargo run -p demo-seed -- --tenant dev-test-01 --seed 42 --modules all

# Print expected digest without hitting APIs
cargo run -p demo-seed -- --tenant dev-test-01 --seed 42 --print-hash

# Write JSON manifest of all created IDs to file
cargo run -p demo-seed -- --tenant dev-test-01 --seed 42 --manifest-out /tmp/manifest.json
```

## CLI Flags

| Flag                     | Env Var              | Default                    | Description |
|--------------------------|----------------------|----------------------------|-------------|
| `--tenant`               | `DEMO_TENANT_ID`     | (required)                 | Tenant namespace |
| `--seed`                 |                      | `42`                       | Deterministic RNG seed |
| `--modules`              |                      | `all`                      | Comma-separated module list |
| `--manifest-out`         |                      | (stdout)                   | Write JSON manifest to file |
| `--print-hash`           |                      | `false`                    | Print digest and exit |
| `--ar-url`               | `AR_BASE_URL`        | `http://localhost:8086`    | AR service URL |
| `--numbering-url`        | `NUMBERING_BASE_URL` | `http://localhost:8120`    | Numbering service URL |
| `--gl-url`               | `GL_BASE_URL`        | `http://localhost:8090`    | GL service URL |
| `--party-url`            | `PARTY_BASE_URL`     | `http://localhost:8098`    | Party service URL |
| `--inventory-url`        | `INVENTORY_BASE_URL` | `http://localhost:8092`    | Inventory service URL |
| `--bom-url`              | `BOM_BASE_URL`       | `http://localhost:8107`    | BOM service URL |
| `--production-url`       | `PRODUCTION_BASE_URL`| `http://localhost:8108`    | Production service URL |
| `--customers`            |                      | `2`                        | AR customer count |
| `--invoices-per-customer`|                      | `3`                        | Invoices per AR customer |

## Manifest JSON Schema

When `--manifest-out` is provided, demo-seed writes a JSON manifest of all
created resource IDs. If omitted, the manifest is printed to stdout after the
digest line.

```json
{
  "tenant_id": "dev-test-01",
  "seed": 42,
  "digest": "sha256-hex-string",
  "users": {
    "admin": { "id": null, "email": "admin@7dsolutions.local" }
  },
  "numbering": {
    "policies": ["purchase-order", "sales-order", "work-order", ...]
  },
  "gl": {
    "accounts": [{"code": "1200", "name": "Raw Materials Inventory"}, ...],
    "fx_rates": [{"rate_id": "uuid", "pair": "USD/EUR"}, ...]
  },
  "parties": {
    "customers": [{"id": "uuid", "legal_name": "The Boeing Company"}, ...],
    "suppliers": [{"id": "uuid", "legal_name": "Bodycote plc"}, ...]
  },
  "inventory": {
    "items": [{"id": "uuid", "sku": "TI64-BAR-001", "make_buy": "buy"}, ...],
    "locations": [{"id": "uuid", "code": "RECV-DOCK"}, ...],
    "uoms": [{"id": "uuid", "code": "EA"}, ...],
    "warehouse_id": "uuid"
  },
  "bom": {
    "boms": [{"id": "uuid", "part_id": "uuid", "revision_id": "uuid", "revision_label": "A"}, ...]
  },
  "production": {
    "workcenters": [{"id": "uuid", "code": "CNC-MILL-01"}, ...],
    "routings": [{"id": "uuid", "item_id": "uuid"}, ...]
  }
}
```

Only sections for seeded modules are included. The `users.admin.id` field is
null because the admin user is created by `seed-dev.sh`, not demo-seed.

## Convenience Script

`scripts/seed-manufacturing.sh` wraps the demo-seed binary with preflight
health checks:

```bash
scripts/seed-manufacturing.sh --tenant dev-test-01
scripts/seed-manufacturing.sh --tenant dev-test-01 --seed 42 --manifest-out /tmp/manifest.json
```

**Relationship to seed-dev.sh:** Run `seed-dev.sh` first (tenant provisioning +
admin + AR), then `seed-manufacturing.sh` (manufacturing modules). They are
complementary.

## Required Services

All services must be running before seeding. The convenience script checks
health endpoints automatically. For manual runs, verify:

- Numbering: `http://localhost:8120/health`
- GL: `http://localhost:8090/api/health`
- Party: `http://localhost:8098/api/health`
- Inventory: `http://localhost:8092/api/health`
- BOM: `http://localhost:8107/api/health`
- Production: `http://localhost:8108/api/health`
- AR: `http://localhost:8086/api/health`

Start the dev stack: `scripts/dev-watch.sh`
