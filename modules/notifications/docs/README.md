# Notifications Module

## Purpose
Delivers outbound communications via email, SMS, and webhooks for the 7D Solutions Platform.

## Ownership Boundaries
**Owns:**
- templates
- notification_preferences
- outbox
- delivery_attempts
- provider_configs

**Does NOT own:**
- Financial logic
- Customer master data
- Invoice state

## Key Principles
- Never drives financial decisions
- No cross-module DB access
- Idempotent delivery on retry
- Event-driven consumption from AR, Payments, Subscriptions

## Architecture References
- **Module Spec:** `docs/architecture/NOTIFICATIONS-MODULE-SPEC.md`
- **OpenAPI Contract:** `contracts/notifications/notifications-v0.1.0.yaml`
- **Event Schemas:** `contracts/events/notifications-*.v1.json`

## Development
```bash
cargo check
cargo test
cargo run
```

Default port: 8089
