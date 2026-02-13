# GL Database Migrations

This directory contains SQL migrations for the GL (General Ledger) module database.

## Migration Files

### 20260212000001_create_gl_schema.sql
Creates the core journal tables:
- `journal_entries` - Journal entry headers with source event tracking for idempotency
- `journal_lines` - Individual debit/credit lines for each entry

**Key Features:**
- `source_event_id` UNIQUE constraint ensures idempotent event processing
- Foreign key from `journal_lines` to `journal_entries` ensures referential integrity
- Non-negative constraints on `debit_minor` and `credit_minor`
- Indexed on tenant_id, posted_at, and source_event_id for query performance

### 20260212000002_create_events_tables.sql
Creates event infrastructure tables:
- `events_outbox` - Transactional outbox pattern for publishing events
- `processed_events` - Idempotent consumer tracking
- `failed_events` - Dead Letter Queue for failed event processing

### 20260213000001_create_accounts_table.sql
Creates the Chart of Accounts (COA) table:
- `accounts` - Defines accounts available for journal line posting
- `account_type` ENUM - Categorizes accounts (asset, liability, equity, revenue, expense)
- `normal_balance` ENUM - Defines normal balance direction (debit or credit)

**Key Features:**
- UNIQUE constraint on (tenant_id, code) prevents duplicate account codes per tenant
- `is_active` flag allows soft-deletion of accounts
- Flat structure (no hierarchy) - simplifies Phase 10 implementation
- Indexed on tenant_id, is_active, and (tenant_id, code) for query performance

### 20260213000002_add_reverses_entry_id.sql
Adds reversal tracking to journal entries:
- `reverses_entry_id` UUID NULL - Links reversal entries to original entries
- Enables audit trail for reversed transactions

### 20260213000003_create_accounting_periods.sql
Creates the accounting periods table for period-aware governance:
- `accounting_periods` - Defines fiscal/accounting periods with closed-period controls
- `period_start` and `period_end` DATE - Define period boundaries
- `is_closed` BOOLEAN - Controls whether posting is allowed in the period

**Key Features:**
- EXCLUDE constraint with btree_gist enforces non-overlapping periods per tenant
- CHECK constraint ensures period_end > period_start
- Multi-tenant isolation - different tenants can have overlapping date ranges
- Indexed on tenant_id, is_closed, and date ranges for query performance
- Enables period-aware posting controls (reject posts to closed periods)

### 20260213000004_create_account_balances.sql
Creates the account balances materialization table:
- `account_balances` - Materialized rollup store for fast trial balance queries
- `debit_total_minor`, `credit_total_minor`, `net_balance_minor` - Cumulative balances in minor units
- Multi-currency support via currency column

**Key Features:**
- UNIQUE constraint on (tenant_id, period_id, account_code, currency) - the materialization grain
- Indexed for fast trial balance queries (tenant + period)
- Account-centric queries supported (balance history across periods)
- last_journal_entry_id provides audit trail
- Enables fast reporting without scanning journal_lines

### 20260213000005_create_period_summary_snapshots.sql
Creates the period summary snapshots table for reporting stability:
- `period_summary_snapshots` - Persists period summaries for fast close previews
- `journal_count`, `line_count` - Activity counts
- `total_debits_minor`, `total_credits_minor` - Monetary totals in minor units
- `checksum` - Optional integrity validation

**Key Features:**
- UNIQUE constraint on (tenant_id, period_id, currency) - the snapshot grain
- Indexed for fast tenant + period lookups
- Supports reporting stability without building a full reporting engine
- Immutable snapshots for period close previews
- created_at timestamp for temporal queries

## Running Migrations

Using sqlx-cli:
```bash
cd modules/gl
sqlx migrate run --database-url postgresql://gl_user:gl_pass@localhost:5438/gl_db
```

## Schema Notes

- All amounts stored as `BIGINT` in minor units (cents) to avoid floating point issues
- Balanced journal entry validation (debits == credits) is enforced at the application layer
- `source_event_id` uniqueness is CRITICAL for preventing duplicate postings
- Timestamps use `TIMESTAMP WITH TIME ZONE` for consistency across timezones
