# Subscriptions Module

## Purpose
Manages recurring billing logic and service agreements for the 7D Solutions Platform.

## Ownership Boundaries
**Owns:**
- Subscriptions
- Billing schedules
- Bill runs
- Proration flags

**Does NOT own:**
- Invoices (calls AR API via OpenAPI)
- Payment state
- Ledger entries

## Key Principles
- Never stores invoice data
- Never stores payment references
- Must not emit financial truth events (AR owns invoices)
- Must not call Payments module directly
- Integration is event-driven + OpenAPI contracts only

## Architecture References
- **Module Spec:** `docs/architecture/SUBSCRIPTIONS-MODULE-SPEC.md`
- **OpenAPI Contract:** `contracts/subscriptions/subscriptions-v1.yaml`
- **Event Schemas:** `contracts/events/subscriptions-*.v1.json`

## Development
```bash
cargo check
cargo test
cargo run
```

Default port: 8087
