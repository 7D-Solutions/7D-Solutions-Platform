# Accounts Payable (AP) Module

The AP module is the authoritative system for vendor-side financial obligations: purchase orders, vendor bills, 3-way matching, payment allocation, and disbursement orchestration.

## Documentation

- **[AP-MODULE-SPEC.md](./docs/AP-MODULE-SPEC.md)**: Product vision, technical specification, and decision log.

## Quick Start

### Build
```bash
./scripts/cargo-slot.sh build -p ap
```

### Test
```bash
./scripts/cargo-slot.sh test -p ap
```

## Status

- **Version**: 0.1.0 (Unproven)
- **Port**: 8093
- **Database**: PostgreSQL (port 5436)
- **Event Bus**: NATS
