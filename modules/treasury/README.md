# Treasury Module

The Treasury module provides a unified view of bank and credit-card positions. It bridges the gap between internal records (GL, AR, AP) and external reality (bank statements) via statement import and reconciliation.

## Documentation

- **[TREASURY-MODULE-SPEC.md](./docs/TREASURY-MODULE-SPEC.md)**: Product vision, technical specification, and decision log.

## Quick Start

### Build
```bash
./scripts/cargo-slot.sh build -p treasury
```

### Test
```bash
./scripts/cargo-slot.sh test -p treasury
```

## Status

- **Version**: 0.1.0 (Unproven)
- **Port**: 8094
- **Database**: PostgreSQL (port 5436)
- **Event Bus**: NATS
