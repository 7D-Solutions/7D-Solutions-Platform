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
