# Accounts Receivable (AR) Module

The AR module is the authoritative system for customer receivables, invoicing, payment processing, and revenue recognition events.

## Documentation

- **[AR-MODULE-SPEC.md](./docs/AR-MODULE-SPEC.md)**: Product vision, technical specification, and decision log.
- **[REVISIONS.md](./REVISIONS.md)**: Revision history for this proven module.

## Quick Start

### Build
```bash
./scripts/cargo-slot.sh build -p ar-rs
```

### Test
```bash
./scripts/cargo-slot.sh test -p ar-rs
```

## Status

- **Version**: 1.0.5 (Proven)
- **Port**: 8086
- **Database**: PostgreSQL (port 5436)
- **Event Bus**: NATS
