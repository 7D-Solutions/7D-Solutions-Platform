# Payments Module Specification

## Mission
Own processor integrations and payment execution.
Never own invoice state.
Never own ledger state.

---

## Owns

- processor_customers
- payment_method_refs
- payment_intents
- captures
- refunds
- dispute_records
- webhook_events (verified)

No PCI storage allowed.
Only processor references.

---

## OpenAPI Surface

### POST /payment-methods
Attach payment method to processor.
Return:
- payment_method_ref

---

### POST /refunds
Request refund by invoice_id or payment_id.

---

## Consumes

- ar.payment.collection.requested

---

## Produces

- payments.payment.succeeded
- payments.payment.failed
- payments.refund.succeeded
- payments.refund.failed
- payments.dispute.opened
- payments.dispute.updated
- payments.dispute.closed

---

## Webhook Handling

- Verify signature
- Store raw webhook
- Emit domain event
- Idempotent on processor event_id

---

## Invariants

1. Never mutate AR DB.
2. Never store raw card data.
3. All processor secrets encrypted at rest.
4. All webhook events stored for audit.

---

## Retry Policy

Payment retry:
- 5 attempts max
- exponential backoff
- emit failed after max attempts

---

## State Machine

intent_created → captured | failed
captured → refunded_partial | refunded_full
