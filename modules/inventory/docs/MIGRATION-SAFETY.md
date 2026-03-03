# Inventory Module — Migration Safety Plan

## Overview

The inventory module uses 20 sequential SQL migrations (`20260218000001` through `20260218000020`).
All migrations are **additive only** — they create tables, indexes, types, and constraints.
None alter or drop existing objects.

## Migration Inventory

| # | Migration | Operations | Reversibility |
|---|-----------|-----------|---------------|
| 001 | create_items | CREATE TABLE items, 2 indexes | DROP TABLE items CASCADE |
| 002 | create_inventory_ledger | CREATE TYPE inv_entry_type, CREATE TABLE inventory_ledger, 5 indexes | DROP TABLE inventory_ledger CASCADE; DROP TYPE inv_entry_type |
| 003 | create_fifo_layers | CREATE TABLE inventory_layers, 2 indexes; CREATE TABLE layer_consumptions, 2 indexes | DROP TABLE layer_consumptions CASCADE; DROP TABLE inventory_layers CASCADE |
| 004 | create_inventory_reservations | CREATE TYPE inv_reservation_status, CREATE TABLE inventory_reservations, indexes | DROP TABLE inventory_reservations CASCADE; DROP TYPE inv_reservation_status |
| 005 | create_item_on_hand_projection | CREATE TABLE item_on_hand, indexes | DROP TABLE item_on_hand CASCADE |
| 006 | create_outbox_and_idempotency | CREATE TABLE inv_outbox, inv_processed_events, inv_idempotency_keys | DROP all three tables CASCADE |
| 007 | create_uoms | CREATE TABLE uoms, item_uom_conversions; ALTER items ADD base_uom_id | DROP TABLE item_uom_conversions, uoms CASCADE; ALTER items DROP base_uom_id |
| 008 | add_tracking_mode_to_items | ALTER items ADD tracking_mode | ALTER items DROP tracking_mode |
| 009 | create_inventory_lots | CREATE TABLE inventory_lots, indexes; ALTER layers ADD lot_id | DROP TABLE inventory_lots CASCADE; ALTER layers DROP lot_id |
| 010 | create_inventory_serial_instances | CREATE TABLE inventory_serial_instances, indexes | DROP TABLE inventory_serial_instances CASCADE |
| 011 | create_status_buckets | CREATE TYPE inv_item_status, CREATE TABLE item_on_hand_by_status | DROP TABLE item_on_hand_by_status CASCADE; DROP TYPE inv_item_status |
| 012 | create_locations | CREATE TABLE locations, indexes | DROP TABLE locations CASCADE |
| 013 | location_aware_ledger_and_projection | ALTER ledger/on_hand ADD location_id, indexes | ALTER DROP columns and indexes |
| 014 | create_status_transfers | CREATE TABLE inv_status_transfers | DROP TABLE inv_status_transfers CASCADE |
| 015 | create_adjustments | CREATE TABLE inv_adjustments | DROP TABLE inv_adjustments CASCADE |
| 016 | create_cycle_count_tables | CREATE TYPE cycle_count_scope, cycle_count_status; CREATE TABLE cycle_count_tasks, cycle_count_lines | DROP both tables CASCADE; DROP TYPE cycle_count_status, cycle_count_scope |
| 017 | create_inv_transfers | CREATE TABLE inv_transfers | DROP TABLE inv_transfers CASCADE |
| 018 | create_reorder_policies | CREATE TABLE reorder_policies | DROP TABLE reorder_policies CASCADE |
| 019 | create_valuation_snapshots | CREATE TABLE inventory_valuation_snapshots, inventory_valuation_lines | DROP both tables CASCADE |
| 020 | create_low_stock_state | CREATE TABLE inv_low_stock_state | DROP TABLE inv_low_stock_state CASCADE |

## Rollback Strategy: Forward-Fix

Since sqlx-migrate does not support down migrations, we use a **forward-fix** approach:

### Full Rollback (drop entire schema)

Execute in reverse order. The `CASCADE` on each DROP ensures FK-dependent objects are removed:

```sql
-- Reverse order: 020 → 001
DROP TABLE IF EXISTS inv_low_stock_state CASCADE;
DROP TABLE IF EXISTS inventory_valuation_lines CASCADE;
DROP TABLE IF EXISTS inventory_valuation_snapshots CASCADE;
DROP TABLE IF EXISTS reorder_policies CASCADE;
DROP TABLE IF EXISTS inv_transfers CASCADE;
DROP TABLE IF EXISTS cycle_count_lines CASCADE;
DROP TABLE IF EXISTS cycle_count_tasks CASCADE;
DROP TYPE IF EXISTS cycle_count_status CASCADE;
DROP TYPE IF EXISTS cycle_count_scope CASCADE;
DROP TABLE IF EXISTS inv_adjustments CASCADE;
DROP TABLE IF EXISTS inv_status_transfers CASCADE;
DROP TABLE IF EXISTS locations CASCADE;
DROP TABLE IF EXISTS item_on_hand_by_status CASCADE;
DROP TYPE IF EXISTS inv_item_status CASCADE;
DROP TABLE IF EXISTS inventory_serial_instances CASCADE;
DROP TABLE IF EXISTS inventory_lots CASCADE;
DROP TABLE IF EXISTS item_uom_conversions CASCADE;
DROP TABLE IF EXISTS uoms CASCADE;
DROP TABLE IF EXISTS inv_idempotency_keys CASCADE;
DROP TABLE IF EXISTS inv_processed_events CASCADE;
DROP TABLE IF EXISTS inv_outbox CASCADE;
DROP TABLE IF EXISTS item_on_hand CASCADE;
DROP TABLE IF EXISTS inventory_reservations CASCADE;
DROP TYPE IF EXISTS inv_reservation_status CASCADE;
DROP TABLE IF EXISTS layer_consumptions CASCADE;
DROP TABLE IF EXISTS inventory_layers CASCADE;
DROP TABLE IF EXISTS inventory_ledger CASCADE;
DROP TYPE IF EXISTS inv_entry_type CASCADE;
DROP TABLE IF EXISTS items CASCADE;
DROP TABLE IF EXISTS _sqlx_migrations CASCADE;
```

### Partial Rollback (specific migration)

To undo a single migration, drop only the objects it created. Always drop in reverse
dependency order within the migration. The `_sqlx_migrations` tracking table must also
be updated:

```sql
-- Example: undo migration 020
DROP TABLE IF EXISTS inv_low_stock_state CASCADE;
DELETE FROM _sqlx_migrations WHERE version = 20260218000020;
```

### Forward-Fix Procedure

For production issues where rollback is undesirable:

1. Create a new migration (next sequence number) that corrects the issue
2. Apply the fix migration forward
3. This preserves data and avoids rollback risk

## Safety Characteristics

- **No destructive operations**: All 20 migrations only CREATE; none DROP or ALTER destructively
- **Idempotent schema**: Each table has IF NOT EXISTS-style safety via unique constraints
- **Tenant isolation by design**: All data tables include `tenant_id` column with indexes
- **Append-only ledger**: inventory_ledger is designed to never update/delete rows
- **FK cascade protection**: Foreign keys prevent orphaned rows

## Verification Procedure

Apply all migrations on a disposable database, then execute the full rollback:

```bash
# 1. Create disposable database
createdb -h localhost -p 5442 -U inventory_user inventory_test_rollback

# 2. Apply all migrations
DATABASE_URL=postgresql://inventory_user:inventory_pass@localhost:5442/inventory_test_rollback \
  cargo test -p inventory-rs --test receipt_integration -- --ignored 2>/dev/null || true
# (migrations run automatically via sqlx::migrate! in test setup)

# 3. Execute full rollback SQL above
# 4. Verify clean state: no inventory tables remain
# 5. Drop disposable database
dropdb -h localhost -p 5442 -U inventory_user inventory_test_rollback
```
