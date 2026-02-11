# Notifications Module Specification

## Mission
Deliver outbound communications.
Never drive financial logic.

---

## Owns

- templates
- notification_preferences
- outbox
- delivery_attempts
- provider_configs

---

## Consumes

- ar.invoice.issued
- ar.payment.applied
- payments.payment.failed
- payments.dispute.opened

---

## Produces

- notifications.delivery.succeeded
- notifications.delivery.failed

---

## OpenAPI

### POST /notifications/test
### GET /templates
### POST /templates

---

## Invariants

1. No financial decisions.
2. No cross-module DB access.
3. Idempotent delivery on retry.
