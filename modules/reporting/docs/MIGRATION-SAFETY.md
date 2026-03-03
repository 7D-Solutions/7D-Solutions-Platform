# Reporting Module: Migration Safety Plan

## Overview

The reporting module has three migrations. All create new tables and indexes
only — no ALTER or DROP operations on existing objects. This makes rollback
straightforward.

## Migrations

### 1. `20260218000001_init.sql`

**Creates:** `reporting_schema_version` (version tracking placeholder)

**Rollback:**
```sql
DROP TABLE IF EXISTS reporting_schema_version;
DELETE FROM _sqlx_migrations WHERE version = 20260218000001;
```

**Risk:** None. No other tables depend on this.

### 2. `20260218000002_create_reporting_caches.sql`

**Creates:**
- `rpt_ingestion_checkpoints` — NATS consumer cursor tracking
- `rpt_trial_balance_cache` — trial balance snapshots
- `rpt_statement_cache` — P&L and balance sheet line items
- `rpt_ar_aging_cache` — AR aging buckets
- `rpt_ap_aging_cache` — AP aging buckets
- `rpt_cashflow_cache` — cash flow statement lines
- `rpt_kpi_cache` — KPI snapshots (MRR, DSO, etc.)

All tables are `rpt_`-prefixed to avoid clashes with source-module schemas.

**Rollback:**
```sql
DROP TABLE IF EXISTS rpt_kpi_cache;
DROP TABLE IF EXISTS rpt_cashflow_cache;
DROP TABLE IF EXISTS rpt_ap_aging_cache;
DROP TABLE IF EXISTS rpt_ar_aging_cache;
DROP TABLE IF EXISTS rpt_statement_cache;
DROP TABLE IF EXISTS rpt_trial_balance_cache;
DROP TABLE IF EXISTS rpt_ingestion_checkpoints;
DELETE FROM _sqlx_migrations WHERE version = 20260218000002;
```

**Risk:** Low. All tables are read-model caches populated by NATS ingestion.
Dropping and re-creating them loses cached data, but the ingestion pipeline
will rebuild it from the event stream on next run. No data loss to
source-of-truth modules.

### 3. `20260222000001_create_forecast_caches.sql`

**Creates:**
- `rpt_payment_history` — historical paid invoices for CDF
- `rpt_open_invoices_cache` — invoice lifecycle tracking

**Rollback:**
```sql
DROP TABLE IF EXISTS rpt_open_invoices_cache;
DROP TABLE IF EXISTS rpt_payment_history;
DELETE FROM _sqlx_migrations WHERE version = 20260222000001;
```

**Risk:** Low. Same as above — cached read-model data rebuilt from events.

## Forward-Fix Strategy

All reporting tables are idempotent caches:
- Every INSERT uses `ON CONFLICT ... DO UPDATE` or `DO NOTHING`.
- Stale data can be corrected by re-running the ingestion pipeline.
- The `/api/reporting/rebuild` admin endpoint triggers a targeted cache rebuild
  for a specific tenant and date range.

If a schema change is needed (e.g., adding a column), the approach is:
1. Add a new migration with `ALTER TABLE ... ADD COLUMN ... DEFAULT ...`.
2. Deploy the new code that reads/writes the new column.
3. No backfill needed — the next ingestion pass will populate new columns.

## Tenant Isolation Guarantee

Every `rpt_*` table has `tenant_id TEXT NOT NULL` as part of its composite
unique constraint and indexed for tenant-scoped queries. All SQL queries bind
`tenant_id` as a parameter — no table-scan or unscoped queries exist.

## Recovery Procedure

If a migration fails mid-apply:
1. Check `_sqlx_migrations` for the partially applied version.
2. Run the rollback SQL above for that specific migration.
3. Fix the migration SQL.
4. Re-run `sqlx migrate run`.
