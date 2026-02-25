# Subscriptions Module

The Subscriptions module is the authoritative system for recurring billing schedules and service agreements. It owns the "when to bill" and "how much to bill" for customer subscriptions.

## Documentation

- **[SUBSCRIPTIONS-MODULE-SPEC.md](./docs/SUBSCRIPTIONS-MODULE-SPEC.md)**: Product vision, technical specification, and decision log.

## Quick Start

### Build
```bash
./scripts/cargo-slot.sh build -p subscriptions-rs
```

### Test
```bash
./scripts/cargo-slot.sh test -p subscriptions-rs
```

## Status

- **Version**: 0.1.0 (Unproven)
- **Port**: 8087
- **Database**: PostgreSQL
- **Event Bus**: NATS
