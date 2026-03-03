# GL Migration Safety: Rollback & Forward-Fix Plan

## Overview

The GL module uses SQLx's `sqlx::migrate!()` macro to run migrations at service startup.
All 19 migrations are **additive** — they use `CREATE TABLE`, `ADD COLUMN IF NOT EXISTS`,
`CREATE INDEX IF NOT EXISTS`, and `DO $$ ... END $$` idempotency guards. No existing
columns or tables are dropped or altered destructively.

## Migration Inventory

| # | Migration | Type | Reversible? | Data Risk |
|---|-----------|------|-------------|-----------|
| 1 | `20260212000001_create_gl_schema` | CREATE TABLE (journal_entries, journal_lines) | DROP TABLE | **HIGH** — contains all posted journals |
| 2 | `20260212000002_create_events_tables` | CREATE TABLE (processed_events, outbox, failed_events, dlq) | DROP TABLE | MEDIUM — idempotency + event state |
| 3 | `20260213000001_create_accounts_table` | CREATE TABLE (accounts) + enum types | DROP TABLE + TYPE | HIGH — chart of accounts |
| 4 | `20260213000002_add_reverses_entry_id` | ADD COLUMN | DROP COLUMN | LOW — nullable FK |
| 5 | `20260213000003_create_accounting_periods` | CREATE TABLE | DROP TABLE | HIGH — period lifecycle |
| 6 | `20260213000004_create_account_balances` | CREATE TABLE | DROP TABLE | HIGH — derived balances |
| 7 | `20260213000005_create_period_summary_snapshots` | CREATE TABLE | DROP TABLE | MEDIUM — derived snapshots |
| 8 | `20260213000006_add_report_indexes` | CREATE INDEX | DROP INDEX | NONE — performance only |
| 9 | `20260214000001_add_period_close_lifecycle` | ADD COLUMN + constraints + indexes | DROP COLUMN/CONSTRAINT | LOW — nullable columns |
| 10 | `20260216000001_add_envelope_metadata_to_outbox` | ADD COLUMN | DROP COLUMN | LOW — nullable metadata |
| 11 | `20260216000002_add_correlation_id_to_journal_entries` | ADD COLUMN | DROP COLUMN | LOW — nullable FK |
| 12 | `20260217000001_create_revrec_tables` | CREATE TABLE (revrec_*) | DROP TABLE | HIGH if contracts exist |
| 13 | `20260217000002_add_schedule_versioning` | ADD COLUMN + indexes | DROP COLUMN | LOW |
| 14 | `20260217000003_create_fx_rates_table` | CREATE TABLE | DROP TABLE | MEDIUM — FX rate history |
| 15 | `20260217000004_create_accrual_tables` | CREATE TABLE (gl_accrual_*) | DROP TABLE | HIGH if accruals posted |
| 16 | `20260217000005_create_accrual_reversals` | CREATE TABLE | DROP TABLE | HIGH if reversals exist |
| 17 | `20260217000006_create_cashflow_classifications` | CREATE TABLE + seed data | DROP TABLE | LOW — reference data |
| 18 | `20260217000007_create_revrec_modifications` | CREATE TABLE | DROP TABLE | LOW — empty until amendments used |
| 19 | `20260218000001_create_close_calendar` | CREATE TABLE | DROP TABLE | LOW — empty until calendar used |
| 20 | `20260218000002_create_close_checklist` | CREATE TABLE (close_checklist_items, close_approvals) | DROP TABLE | MEDIUM if checklists active |
| 21 | `20260218000003_period_reopen_audit` | CREATE TABLE | DROP TABLE | LOW |

## Rollback Procedures

### Strategy: Forward-Fix Preferred

Since all migrations are additive, **rollback is rarely needed**. The preferred approach:

1. **Fix the bug in code** — the schema is usually correct
2. **Add a new migration** to correct any schema issues
3. **Never DROP production tables** — data loss is unrecoverable

### If Rollback Is Required (Disposable/Test DB Only)

```sql
-- 1. Check current migration state
SELECT * FROM _sqlx_migrations ORDER BY installed_on DESC LIMIT 5;

-- 2. Delete the migration record (ONLY on disposable DB)
DELETE FROM _sqlx_migrations WHERE version = 20260218000003;

-- 3. Run the reverse DDL manually (example for migration 21)
DROP TABLE IF EXISTS period_reopen_requests CASCADE;

-- 4. Restart service — sqlx::migrate!() will not re-run a present migration
```

### Rollback Ordering (Reverse FK Dependencies)

If rolling back multiple migrations, follow this order (reverse of creation):

1. Drop dependent tables first (reopen → checklist → calendar → modifications → reversals → accruals)
2. Drop independent tables next (fx_rates, revrec_schedules)
3. Drop core tables last (account_balances → periods → accounts → journal_lines → journal_entries)

### Emergency Recovery: Corrupted Migration State

```sql
-- If _sqlx_migrations shows a migration as applied but the table doesn't exist:
-- 1. Remove the migration record
DELETE FROM _sqlx_migrations WHERE version = <MIGRATION_VERSION>;

-- 2. Restart service — migration will re-run (all migrations are idempotent)
```

## Validation Procedure (Disposable DB)

Run this against a **disposable test database** to validate migration safety:

```bash
# 1. Start a fresh database
docker compose -f docker-compose.test.yml up -d gl-db

# 2. Run all migrations (via service startup)
DATABASE_URL="postgres://gl_user:gl_pass@localhost:5438/gl_db" \
  cargo run -p gl-rs

# 3. Verify all tables exist
psql "postgres://gl_user:gl_pass@localhost:5438/gl_db" -c "
  SELECT tablename FROM pg_tables
  WHERE schemaname = 'public'
  ORDER BY tablename;
"

# 4. Verify migration count matches expected
psql "postgres://gl_user:gl_pass@localhost:5438/gl_db" -c "
  SELECT COUNT(*) as migration_count FROM _sqlx_migrations;
"
# Expected: 21

# 5. Run full test suite to verify schema is functionally correct
./scripts/cargo-slot.sh test -p gl-rs --test tenant_boundary_concurrency_test -- --test-threads=1

# 6. Destroy disposable DB
docker compose -f docker-compose.test.yml down -v
```

## Invariants

1. **All migrations are idempotent** — `IF NOT EXISTS` / `ON CONFLICT DO NOTHING` throughout
2. **All tables have tenant_id** — tenant scoping is enforced at the schema level
3. **No destructive DDL** — no DROP, ALTER TYPE, or ALTER COLUMN in any migration
4. **Foreign keys cascade where appropriate** — journal_lines → journal_entries
5. **Unique constraints protect idempotency** — source_event_id, idempotency_key fields
