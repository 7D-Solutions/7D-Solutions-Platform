# Maintenance Module — Migration Rollback / Forward-Fix Plan

## Overview

All 6 migrations are **additive DDL** (CREATE TABLE, ALTER TABLE ADD COLUMN, CREATE INDEX).
No data transformations, no column drops, no constraint modifications on existing columns.
Each migration is independently reversible with a simple DROP.

## Migration Inventory

| # | File | Operation | Reversible? |
|---|------|-----------|-------------|
| 1 | `20260224000001_create_events_outbox.sql` | CREATE TABLE events_outbox, processed_events + indexes | DROP TABLE |
| 2 | `20260224000002_core_tables.sql` | CREATE TABLE maintainable_assets, meter_types, maintenance_plans, maintenance_plan_assignments, work_orders, wo_counters + indexes | DROP TABLE (reverse order) |
| 3 | `20260224000003_meter_readings.sql` | CREATE TABLE meter_readings + indexes | DROP TABLE |
| 4 | `20260224000004_parts_and_labor.sql` | CREATE TABLE work_order_parts, work_order_labor + indexes | DROP TABLE |
| 5 | `20260224000005_add_due_notified_at.sql` | ALTER TABLE ADD COLUMN + index | ALTER TABLE DROP COLUMN |
| 6 | `20260224000006_tenant_config.sql` | CREATE TABLE maintenance_tenant_config | DROP TABLE |

## Rollback Procedure

Migrations must be rolled back in **reverse order** due to foreign key dependencies.

```
-- Step 6: DROP maintenance_tenant_config
DROP TABLE IF EXISTS maintenance_tenant_config;

-- Step 5: DROP due_notified_at column + index
DROP INDEX IF EXISTS idx_plan_assignments_due_candidates;
ALTER TABLE maintenance_plan_assignments DROP COLUMN IF EXISTS due_notified_at;

-- Step 4: DROP parts & labor
DROP TABLE IF EXISTS work_order_labor;
DROP TABLE IF EXISTS work_order_parts;

-- Step 3: DROP meter readings
DROP TABLE IF EXISTS meter_readings;

-- Step 2: DROP core tables (reverse FK order)
DROP TABLE IF EXISTS wo_counters;
DROP TABLE IF EXISTS work_orders;
DROP TABLE IF EXISTS maintenance_plan_assignments;
DROP TABLE IF EXISTS maintenance_plans;
DROP TABLE IF EXISTS meter_types;
DROP TABLE IF EXISTS maintainable_assets;

-- Step 1: DROP outbox tables
DROP TABLE IF EXISTS processed_events;
DROP TABLE IF EXISTS events_outbox;
```

After rollback, delete the corresponding rows from `_sqlx_migrations`:
```sql
DELETE FROM _sqlx_migrations WHERE version >= 20260224000001 AND version <= 20260224000006;
```

## Forward-Fix Strategy

Since all migrations are additive, forward-fix is preferred over rollback:

1. **Schema bug**: Add a new migration (000007+) that corrects the issue (ALTER TABLE, CREATE INDEX, etc.).
2. **Bad data from application bug**: Fix the application code, deploy, then run a data repair migration.
3. **Constraint violation**: If a new constraint would reject existing data, add it as NOT VALID first, then VALIDATE CONSTRAINT separately.

## Tenant Isolation in Migrations

Every table that stores business data includes a `tenant_id TEXT NOT NULL` column.
All query indexes include `tenant_id` as the leading column.
The `events_outbox` and `processed_events` tables are infrastructure (not tenant-scoped).

## Risk Assessment

- **Data loss risk**: None. All operations are CREATE/ADD — no DROP, no ALTER TYPE, no column removal.
- **Lock risk**: Low. CREATE TABLE and CREATE INDEX take brief locks. No long-running ALTER on existing large tables.
- **Rollback data loss**: Rolling back destroys all maintenance data. Only do this if the module has not yet stored production data, or if the data can be re-imported.
