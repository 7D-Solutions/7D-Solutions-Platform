# Payments Health Audit — 2026-04-12

**Bead:** bd-b6z7h  
**Auditor:** PurpleCliff  
**Date:** 2026-04-12

---

## Summary

The Payments module had no end-to-end canary test. A regression in the payment
lifecycle (customer → charge → refund) would go undetected until a customer
reported a failed charge. This audit documents the gap, the current health
of the payments path, and what the new canary covers.

---

## Current Payments Health

### What works (verified by existing tests)

| Area | Test | Status |
|------|------|--------|
| Lifecycle state machine (ATTEMPTING → SUCCEEDED) | `payments_lifecycle_e2e` | ✅ Pass |
| Lifecycle retry path (ATTEMPTING → FAILED_RETRY → ATTEMPTING → SUCCEEDED) | `payments_lifecycle_e2e` | ✅ Pass |
| Terminal guard (SUCCEEDED rejects all outgoing transitions) | `payments_lifecycle_e2e` | ✅ Pass |
| Partial payment allocation | `payments_lifecycle_e2e` | ✅ Pass |
| Multi-currency payment allocation (EUR) | `payments_lifecycle_e2e` | ✅ Pass |
| Checkout session HTTP routes | `payments_http_smoke` | ✅ Pass |
| Webhook signature rejection | `payments_http_smoke` | ✅ Pass |
| Outbox atomicity | `payments_outbox_atomicity_e2e` | ✅ Pass |
| UNKNOWN protocol (cross-module) | `cross_module_payment_unknown_e2e` | ✅ Pass |

### Known gaps at time of audit

| Gap | Severity | Resolution |
|-----|----------|------------|
| No happy-path canary covering the full customer → charge → refund cycle | High | Resolved by this bead (bd-b6z7h) |
| No CI gate specifically testing the Payments + AR DB connection pool under load | High | Resolved by adding `e2e-payments-health-canary` CI job |
| Payments docker healthcheck startup detection interval was unspecified in CI | Medium | Resolved: new CI job uses `--health-interval 10s` |

---

## What the Canary Covers

File: `e2e-tests/tests/payments_health_canary_e2e.rs`

The canary runs the full happy-path in sequence against real AR-postgres
and Payments-postgres (no mocks, no Tilled API calls):

1. **Create customer** — inserts an AR customer directly in `ar_customers`.
2. **Add payment method** — inserts a card payment method in `ar_payment_methods`,
   marked as default. Simulates the customer onboarding step.
3. **Create charge (payment attempt)** — inserts a `payment_attempts` row in
   ATTEMPTING state and immediately transitions it to SUCCEEDED via the lifecycle
   function. Simulates a PSP confirmation.
4. **Assert charge status = succeeded** — reads back the attempt status from
   Payments-postgres and asserts it equals `succeeded`.
5. **Create AR charge record** — inserts an `ar_charges` row with `status=succeeded`
   and a `tilled_charge_id`. Simulates the charge domain record created by the
   AR webhook handler after PSP confirmation.
6. **Create refund** — inserts an `ar_refunds` row with `status=succeeded` against
   the charge. Simulates a refund processed immediately by the PSP.
7. **Assert refund status = succeeded** — reads back the refund status from
   AR-postgres and asserts it equals `succeeded`.

The entire sequence is wrapped in a **60-second timeout**. If the test exceeds
60 seconds, it fails with:

```
FAIL: payments health canary timed out after 60s — likely DB pool starvation or service hang
```

This catches:
- DB connection pool exhaustion (pool acquire blocks indefinitely)
- Deadlocks on payment or AR tables
- Migration hangs
- Service-level hangs introduced by a code change

---

## Limits / What the Canary Does Not Cover

- **Tilled API integration**: the canary does not call the real Tilled payment
  processor. That is covered by `payments_http_smoke` (which requires a live
  Tilled sandbox config in env vars).
- **Webhook processing**: the canary does not exercise the webhook handler or
  the event bus pipeline. Those are covered by `payments_outbox_atomicity_e2e`
  and `cross_module_payment_unknown_e2e`.
- **Reconciliation**: UNKNOWN → reconciled path is not in this canary. That is
  covered by `payments_lifecycle_e2e` (retry path) and the reconciliation tests.

---

## CI Job

The canary runs as `e2e-payments-health-canary` in `.github/workflows/ci.yml`.

- **Trigger**: every PR (needs: `[contract-tests]`)
- **Services**: isolated AR-postgres and Payments-postgres, each with
  `--health-interval 10s` so CI catches startup failures within 60s total
  (6 retries × 10s = 60s max health probe window).
- **Test timeout**: 60 seconds (enforced by `tokio::time::timeout` inside the test)

---

## Recommendations

1. **Wire Tilled sandbox env vars in CI** so `payments_http_smoke` runs on
   every PR rather than skipping when `JWT_PRIVATE_KEY_PEM` is absent. Track
   as a separate bead.
2. **Add a pool saturation test** that opens N concurrent connections to the
   Payments DB and measures acquire latency under load. Track as a separate bead.
