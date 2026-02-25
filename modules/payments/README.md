# Payments Module

The Payments module is the authoritative system for payment execution across all tenant applications. It owns the relationship with the payment service provider (PSP) and ensures deterministic payment collection.

## Documentation

- **[PAYMENTS-MODULE-SPEC.md](./docs/PAYMENTS-MODULE-SPEC.md)**: Product vision, technical specification, and decision log.
- **[REVISIONS.md](./REVISIONS.md)**: Revision history for this proven module.

## Quick Start

### Build
```bash
./scripts/cargo-slot.sh build -p payments-rs
```

### Test
```bash
./scripts/cargo-slot.sh test -p payments-rs
```

## Status

- **Version**: 1.1.5 (Proven)
- **Port**: 8088
- **Database**: PostgreSQL
- **Event Bus**: NATS
