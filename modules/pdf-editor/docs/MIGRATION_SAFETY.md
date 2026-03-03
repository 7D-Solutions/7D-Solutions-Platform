# PDF Editor Module — Migration Rollback / Forward-Fix Plan

## Overview

Both migrations are **additive DDL** (CREATE TABLE + CREATE INDEX).
No data transformations, no column drops, no constraint modifications on existing columns.
Each migration is independently reversible with a simple DROP.

## Migration Inventory

| # | File | Operation | Reversible? |
|---|------|-----------|-------------|
| 1 | `20260224000001_create_events_outbox.sql` | CREATE TABLE events_outbox, processed_events + indexes | DROP TABLE |
| 2 | `20260224000002_pdf_editor_schema.sql` | CREATE TABLE form_templates, form_fields, form_submissions + indexes | DROP TABLE (reverse FK order) |

## Rollback Procedure

Migrations must be rolled back in **reverse order** due to foreign key dependencies.

```sql
-- Step 2: DROP pdf-editor business tables (reverse FK order)
DROP TABLE IF EXISTS form_submissions;
DROP TABLE IF EXISTS form_fields;
DROP TABLE IF EXISTS form_templates;

-- Step 1: DROP outbox infrastructure tables
DROP TABLE IF EXISTS processed_events;
DROP TABLE IF EXISTS events_outbox;
```

After rollback, delete the corresponding rows from `_sqlx_migrations`:
```sql
DELETE FROM _sqlx_migrations WHERE version >= 20260224000001 AND version <= 20260224000002;
```

## Forward-Fix Strategy

Since all migrations are additive, forward-fix is preferred over rollback:

1. **Schema bug**: Add a new migration (000003+) that corrects the issue (ALTER TABLE, CREATE INDEX, etc.).
2. **Bad data from application bug**: Fix the application code, deploy, then run a data repair migration.
3. **Constraint violation**: If a new constraint would reject existing data, add it as NOT VALID first, then VALIDATE CONSTRAINT separately.

## Tenant Isolation in Migrations

Every business table includes a `tenant_id TEXT NOT NULL` column:
- `form_templates` — indexed on `(tenant_id)` and `(tenant_id, name)`
- `form_submissions` — indexed on `(tenant_id)` and `(tenant_id, status)`

The `form_fields` table is scoped indirectly via its FK to `form_templates`.
The `events_outbox` and `processed_events` tables are infrastructure (tenant_id present but used for tracing, not isolation).

## Risk Assessment

- **Data loss risk**: None. All operations are CREATE — no DROP, no ALTER TYPE, no column removal.
- **Lock risk**: Low. CREATE TABLE and CREATE INDEX take brief locks. No long-running ALTER on existing large tables.
- **Rollback data loss**: Rolling back destroys all PDF editor data. Only do this if the module has not yet stored production data, or if the data can be re-imported.
