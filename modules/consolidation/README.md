# Consolidation Module

Multi-entity financial consolidation engine. Aggregates trial balances from subsidiary GL databases, applies elimination rules, COA mappings, and FX translation to produce consolidated financial statements.

## Architecture

- **Language**: Rust
- **Framework**: Axum
- **Database**: PostgreSQL (port 5446)
- **Port**: 8105 (default)

## Key Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/consolidation/groups/{group_id}/consolidate` | Run consolidation |
| GET | `/api/consolidation/groups/{group_id}/trial-balance` | Consolidated trial balance |
| CRUD | `/api/consolidation/groups` | Consolidation groups |
| CRUD | `/api/consolidation/groups/{group_id}/entities` | Group entities |
| CRUD | `/api/consolidation/groups/{group_id}/coa-mappings` | COA mappings |
| CRUD | `/api/consolidation/groups/{group_id}/elimination-rules` | Elimination rules |
| PUT | `/api/consolidation/groups/{group_id}/fx-policies` | FX translation policies |
| POST | `/api/consolidation/groups/{group_id}/intercompany-match` | Intercompany matching |
| POST | `/api/consolidation/groups/{group_id}/eliminations` | Post elimination entries |

## Database Tables

- `csl_groups` — consolidation group definitions
- `csl_group_entities` — entities belonging to each group
- `csl_coa_mappings` — chart-of-accounts mappings between entities
- `csl_elimination_rules` — intercompany elimination rules
- `csl_fx_policies` — FX translation policies per group
- `csl_trial_balance_cache` — cached consolidated trial balances
- `csl_statement_cache` — cached consolidated statements
- `csl_elimination_postings` — posted elimination journal entries

## Events

None. Consolidation is a read-aggregation service that pulls trial balances from GL via HTTP.

## Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | Yes | — | PostgreSQL connection string |
| `GL_BASE_URL` | No | `http://localhost:8080` | GL service URL for fetching trial balances |
| `HOST` | No | `0.0.0.0` | Bind address |
| `PORT` | No | `8105` | HTTP port |
| `ENV` | No | `development` | Environment name |
| `CORS_ORIGINS` | No | — | Comma-separated allowed origins |

## Documentation

- **[CONSOLIDATION-MODULE-SPEC.md](./docs/CONSOLIDATION-MODULE-SPEC.md)**: Full specification

## Development

```bash
./scripts/cargo-slot.sh build -p consolidation
./scripts/cargo-slot.sh test -p consolidation
```
