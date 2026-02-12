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

- `GET /health` - Health check endpoint

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

## Docker

The service runs in Docker Compose as `gl-rs` with a dedicated PostgreSQL instance `7d-gl-postgres`.
