# Shipping-Receiving Migration Safety

## Tables

| Table | Purpose | Has tenant_id |
|-------|---------|:---:|
| `shipments` | Core shipment records (inbound/outbound) | Yes |
| `shipment_lines` | Line items per shipment | Yes |
| `sr_events_outbox` | Transactional outbox for event publishing | Yes |
| `sr_processed_events` | Idempotent consumer tracking | No (keyed by event_id) |

## Migration Inventory

| # | File | Description |
|---|------|-------------|
| 1 | `20260225000001_create_events_outbox.sql` | Outbox + processed events (initial schema) |
| 2 | `20260225000002_create_shipments.sql` | Shipments + shipment_lines tables, indexes, constraints |
| 3 | `20260225000003_create_processed_events.sql` | Fixes sr_processed_events schema (drops migration 1 version, recreates with event_id PK) |

## Known Issues (Resolved)

Migration 1 created `sr_processed_events` with a `processor NOT NULL` column and `BIGSERIAL` primary key. Migration 3 intended to replace it with a simpler schema (`event_id UUID` as PK, no processor). On a fresh database, migration 3 would fail because the table already existed from migration 1.

**Fix (bd-227n8):** Migration 3 now performs `DROP TABLE IF EXISTS sr_processed_events CASCADE` before `CREATE TABLE`, ensuring the correct schema is always applied regardless of database state.

## Forward-Fix Rollback Procedure

If a migration causes issues in staging/production, use forward-fix (new migration) rather than editing existing migrations. For a full rollback to clean slate:

```sql
-- Execute in reverse dependency order
DROP TABLE IF EXISTS shipment_lines CASCADE;
DROP TABLE IF EXISTS shipments CASCADE;
DROP TABLE IF EXISTS sr_processed_events CASCADE;
DROP TABLE IF EXISTS sr_events_outbox CASCADE;
DROP TABLE IF EXISTS _sqlx_migrations CASCADE;
```

Then re-apply all migrations via `sqlx migrate run` or application startup.

## Automated Verification

The `migration_safety_test.rs` integration test suite validates:

1. **Clean apply** — All migrations apply without error on a fresh database
2. **Forward-fix rollback** — Drop all tables, re-apply successfully
3. **Tenant isolation** — All data tables have `tenant_id` column
4. **Schema correctness** — `sr_processed_events` has correct PK and no stale columns
5. **Constraint integrity** — Direction/status CHECK constraints and quantity non-negativity enforced
