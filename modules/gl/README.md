# GL Module (General Ledger)

The GL module is responsible for consuming GL posting requests and creating balanced journal entries in the accounting system.

## Overview

This service consumes `gl.events.posting.requested` events from NATS and persists balanced journal entries to the GL database, ensuring idempotency and proper error handling.

## Architecture

- **Language**: Rust
- **Framework**: Axum (HTTP server)
- **Database**: PostgreSQL (port 5438)
- **Message Bus**: NATS
- **Port**: 8090

## Configuration

The service is configured via environment variables:

- `DATABASE_URL` - PostgreSQL connection string (required)
- `BUS_TYPE` - Event bus type: `inmemory` or `nats` (default: `inmemory`)
- `NATS_URL` - NATS server URL (default: `nats://localhost:4222`)
- `HOST` - Server bind address (default: `0.0.0.0`)
- `PORT` - Server port (default: `8090`)

## Endpoints

- `GET /api/health` - Health check endpoint
- `GET /api/gl/trial-balance` - Trial balance API (query params: tenant_id, period_id, currency)

## Database Schema

The GL module uses the following tables:

- `journal_entries` - Journal entry headers with source tracking and idempotency
- `journal_lines` - Individual debit/credit lines for each journal entry
- `events_outbox` - Outbound events to be published
- `processed_events` - Deduplicated inbound event tracking
- `failed_events` - DLQ for events that fail processing

## Development

Build the service:
```bash
cargo build
```

Run locally:
```bash
cargo run
```

## Admin Tools

### rebuild_balances - Audit Recovery Tool

The `rebuild_balances` tool provides deterministic balance recomputation from journal entries. This is an admin-only tool for audit integrity and recovery scenarios.

**Purpose:**
- Rebuild account balances from journal entry source-of-truth
- Recover from balance corruption or data migration
- Verify balance integrity after system changes

**Usage:**
```bash
# Build and run locally (requires DATABASE_URL env var)
cd modules/gl
cargo run --bin rebuild_balances -- \
  --tenant TENANT_ID \
  --from 2026-01-01 \
  --to 2026-12-31

# Or build release and run
cargo build --release --bin rebuild_balances
./target/release/rebuild_balances \
  --tenant tenant_123 \
  --from 2026-01-01 \
  --to 2026-12-31
```

**Safety Features:**
- **Tenant isolation**: Operates on one tenant at a time
- **Transactional**: Each period is rebuilt in a single transaction
- **Deterministic**: Same journal entries always produce same balances
- **Batched**: Processes one period at a time to avoid long locks
- **Idempotent**: Safe to run multiple times

**How it works:**
1. Finds all accounting periods that overlap with the date range
2. For each period:
   - Fetches all journal entries in the period
   - Deletes existing balances for the period (in transaction)
   - Recomputes balances by aggregating journal lines
   - Inserts new balances (in transaction)
3. All operations are logged for audit trail

**When to use:**
- After data migration or import
- To recover from suspected balance corruption
- To verify balance integrity after schema changes
- For audit investigations

**Production considerations:**
- Run during maintenance windows (writes to account_balances)
- Test on staging environment first
- Backup database before running
- Monitor database load during execution

## Docker

The service runs in Docker Compose as `gl-rs` with a dedicated PostgreSQL instance `gl-postgres`.
