# Canonical Financial Flow — TrashTech Billing (Composed Capability)

## Purpose
Define the canonical, end-to-end flow for TrashTech billing using primitive modules.
This is **the** reference flow for billing, collections, and GL posting.

**Locked model**
- **Option A:** AR drives collection via event command.
- **Payments executes** processor interaction.
- **AR applies** payment results to invoices.
- **AR emits** GL posting requests; GL responds accepted/rejected.

---

## Actors / Modules
- AUTH: `platform/identity-auth`
- AR: `modules/ar`
- SUBS: `modules/subscriptions` (planned)
- PAY: `modules/payments` (planned)
- NOTIF: `modules/notifications` (planned)
- GL: `modules/gl` (future) or external accounting module

---

## Flow 0 — Setup (Tenant/User)
1. AUTH provisions tenant/users/roles (out of billing scope).

---

## Flow 1 — Create Customer + Payment Method Reference
**Goal:** AR knows who to invoice; Payments knows how to collect.

1. UI/Product calls AR to create customer (billing-facing customer record).
2. UI/Product calls Payments to attach payment method to processor customer.
3. Payments returns `payment_method_ref` (opaque reference).
4. UI/Product (or an AR endpoint) stores `payment_method_ref` in AR customer profile.

**Rules**
- AR stores references only (no secrets).
- Payments stores processor ids and webhook truth.

---

## Flow 2 — Create Subscription (Recurring Service)
1. UI/Product calls SUBS OpenAPI to create subscription linked to `ar_customer_id`.
2. SUBS computes schedule and stores next bill date.

---

## Flow 3 — Bill Run → Invoice Issuance
**Trigger:** scheduled job or manual run.

1. SUBS identifies billable subscriptions for a period.
2. SUBS calls AR OpenAPI: create/issue invoice with line items and service period metadata.
3. AR creates invoice and emits:
   - `ar.invoice.issued` (fact)

**Rules**
- SUBS never creates invoices in its own DB.
- AR is sole authority on invoice status.

---

## Flow 4 — Collection (Option A Locked)
**Trigger:** invoice issued or explicit "collect now" policy.

1. AR emits command event:
   - `ar.payment.collection.requested`
   containing:
   - tenant_id
   - invoice_id
   - ar_customer_id
   - amount (minor units)
   - currency
   - payment_method_ref (or customer default ref pointer)

2. PAY consumes `ar.payment.collection.requested`.
3. PAY creates processor intent + capture (MVP: immediate capture).
4. PAY emits one of:
   - `payments.payment.succeeded` (fact)
   - `payments.payment.failed` (fact)

5. AR consumes payment result event:
   - On succeeded: apply payment allocation; mark invoice paid/partial as appropriate.
   - On failed: record failure; invoice remains open.

6. AR emits:
   - `ar.payment.applied` (fact) when applied successfully

---

## Flow 5 — GL Posting (Event-Driven Only)
**Triggers (minimum)**
- Invoice issued
- Payment applied
- Refund recorded (accounting artifact)
- Write-off
- Dispute adjustments

For each trigger AR emits:
- `gl.posting.requested`

GL responds:
- `gl.posting.accepted`
- `gl.posting.rejected`

**Critical rule**
- If GL rejects: AR **must not** roll back invoice/payment truth silently.
- AR records posting failure and surfaces reconciliation queue.

---

## Flow 6 — Refunds
1. UI/Product requests refund via PAY OpenAPI.
2. PAY executes refund and emits:
   - `payments.refund.succeeded|failed`
3. AR consumes refund result and records accounting artifact:
   - credit/refund adjustment applied to invoice/ledger
4. AR emits `gl.posting.requested` for refund accounting.

---

## Flow 7 — Disputes / Chargebacks
1. PAY ingests processor webhook; verifies and emits `payments.dispute.*`
2. AR consumes and records dispute as financial artifact:
   - link to invoice/payment allocation
   - update dispute lifecycle
3. AR emits `gl.posting.requested` for any required accounting adjustments.

---

## Idempotency & Correlation
- Every event uses the envelope standard in `contracts/events/README.md`.
- Consumers must:
  - de-duplicate by `event_id`
  - enforce state machine transitions
  - record correlation_id + causation_id for auditability

---

## What This Flow Prohibits
- Payments changing invoice state directly
- Subscriptions storing invoice truth
- Notifications making financial decisions
- Any module writing directly to GL DB
- Cross-module DB reads/writes
