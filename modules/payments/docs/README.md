# Payments Module

## Purpose
Owns processor integrations and payment execution for the 7D Solutions Platform.

## Ownership Boundaries
**Owns:**
- processor_customers
- payment_method_refs
- payment_intents
- captures
- refunds
- dispute_records
- webhook_events (verified)

**Does NOT own:**
- Invoices (AR owns invoice state)
- Ledger entries (GL owns ledger)
- Customer master data (AR owns customers)

## Key Principles
- Never mutates AR database
- Never stores raw card data (PCI-DSS scope minimization)
- All processor secrets encrypted at rest
- All webhook events stored for audit
- Idempotent webhook processing

## Architecture References
- **Module Spec:** `docs/architecture/PAYMENTS-MODULE-SPEC.md`
- **OpenAPI Contract:** `contracts/payments/payments-v0.1.0.yaml`
- **Event Schemas:** `contracts/events/payments-*.v1.json`

## Development
```bash
cargo check
cargo test
cargo run
```

Default port: 8088
