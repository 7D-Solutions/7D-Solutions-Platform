# Subscriptions Module Specification (v0.1.x)

## Mission
Own recurring billing logic and service agreements.
Never own invoice truth.
Never own financial ledger state.

Subscriptions schedules.
AR accounts.

---

## Owns

- subscriptions
- subscription_plans
- subscription_items
- billing_schedule
- bill_run_state
- proration_flags
- service_period_definitions

All records must include:
- tenant_id
- created_at
- updated_at

---

## Does NOT Own

- invoices
- balances
- payment state
- payment methods
- ledger
- GL entries

---

## OpenAPI Surface

### POST /subscriptions
Create subscription
Request:
- tenant_id
- ar_customer_id
- plan_id
- schedule (monthly/weekly/custom)
- start_date
- price_minor
- currency

Response:
- subscription_id
- next_bill_date

---

### POST /subscriptions/{id}/pause
### POST /subscriptions/{id}/resume
### GET /subscriptions/{id}
### GET /subscriptions?customer_id=

---

### POST /bill-runs/execute
Triggers billing cycle.

Logic:
1. Find subscriptions due.
2. Generate invoice payload.
3. Call AR OpenAPI:
   POST /invoices

Subscriptions never writes invoices.

---

## Events Produced

- subscriptions.created
- subscriptions.paused
- subscriptions.resumed
- subscriptions.billrun.executed

---

## Invariants

1. No invoice data stored.
2. No payment references stored.
3. Must not emit financial truth events.
4. Must not call Payments.

---

## TrashTech Requirements

- Weekly pickup schedule
- Service period metadata
- Proration disabled in MVP (flag exists)

---

## State Machine

active → paused → resumed → cancelled

Cancelled cannot resume.

---

## Idempotency

Bill runs must:
- Track bill_run_id
- Prevent duplicate invoice creation

---

## Versioning

SemVer independent.
Breaking change if:
- schedule format changes
- plan schema changes
