# App ID Scoping Audit - Billing Module

**Auditor:** BrownIsland
**Date:** 2026-01-31
**Purpose:** Verify all billingPrisma queries properly scope by app_id for multi-tenant data isolation

## Executive Summary

**Critical Issues Found:** 5 queries missing app_id scoping
**Warnings:** 3 queries with indirect scoping (acceptable pattern)
**Passed:** 24 queries properly scoped

## Critical Issues (Requires Immediate Fix)

### 1. WebhookService.js - Webhook Record Updates (3 instances)

**Problem:** Webhook updates use `event_id` as the only WHERE clause, allowing cross-tenant access.

**Locations:**
- `WebhookService.js:37-40` - Failed signature webhook update
- `WebhookService.js:49-52` - Processed webhook update
- `WebhookService.js:58-61` - Failed processing webhook update

**Current Code:**
```javascript
await billingPrisma.billing_webhooks.update({
  where: { event_id: event.id },
  data: { status: 'failed', error: 'Invalid signature', processed_at: new Date() }
});
```

**Risk:** High - An attacker knowing another tenant's event_id could potentially manipulate webhook status.

**Recommendation:** Add app_id to unique constraint and WHERE clause:
```javascript
await billingPrisma.billing_webhooks.update({
  where: {
    event_id_app_id: {
      event_id: event.id,
      app_id: appId
    }
  },
  data: { status: 'failed', error: 'Invalid signature', processed_at: new Date() }
});
```

**Schema Change Required:**
```prisma
model billing_webhooks {
  @@unique([event_id, app_id], name: "event_id_app_id")
}
```

---

### 2. WebhookService.js - Refund Lookup and Update

**Problem:** Refund queries use `tilled_refund_id` only, no app_id scoping.

**Locations:**
- `WebhookService.js:235-239` - Refund lookup
- `WebhookService.js:243-253` - Refund update

**Current Code:**
```javascript
const existingRefund = await billingPrisma.billing_refunds.findFirst({
  where: {
    tilled_refund_id: tilledRefund.id
  }
});

await billingPrisma.billing_refunds.update({
  where: {
    tilled_refund_id: tilledRefund.id
  },
  data: { ... }
});
```

**Risk:** Medium - Cross-tenant refund data access if tilled_refund_id is predictable.

**Recommendation:** Add app_id to WHERE clause:
```javascript
const existingRefund = await billingPrisma.billing_refunds.findFirst({
  where: {
    tilled_refund_id: tilledRefund.id,
    app_id: appId
  }
});
```

---

### 3. WebhookService.js - Dispute Upsert

**Problem:** Dispute upsert uses `tilled_dispute_id` only, no app_id scoping.

**Location:** `WebhookService.js:326-356`

**Current Code:**
```javascript
await billingPrisma.billing_disputes.upsert({
  where: {
    tilled_dispute_id: tilledDispute.id
  },
  update: { ... },
  create: { app_id: appId, ... }
});
```

**Risk:** Medium - Cross-tenant dispute manipulation.

**Recommendation:** Add app_id to unique constraint and WHERE clause:
```javascript
await billingPrisma.billing_disputes.upsert({
  where: {
    tilled_dispute_id_app_id: {
      tilled_dispute_id: tilledDispute.id,
      app_id: appId
    }
  },
  update: { ... },
  create: { app_id: appId, ... }
});
```

**Schema Change Required:**
```prisma
model billing_disputes {
  @@unique([tilled_dispute_id, app_id], name: "tilled_dispute_id_app_id")
}
```

---

### 4. PaymentMethodService.js - Payment Method Soft Delete

**Problem:** Soft delete uses `tilled_payment_method_id` only.

**Location:** `PaymentMethodService.js:190-196`

**Current Code:**
```javascript
await billingPrisma.billing_payment_methods.update({
  where: { tilled_payment_method_id: tilledPaymentMethodId },
  data: {
    deleted_at: new Date(),
    is_default: false
  }
});
```

**Risk:** Low - Function already verifies ownership via findFirst with app_id (line 163-169), but update still vulnerable to TOCTOU race.

**Recommendation:** Add app_id verification to update or use id-based update:
```javascript
// Option 1: Use the verified paymentMethod.id instead
await billingPrisma.billing_payment_methods.update({
  where: { id: paymentMethod.id },
  data: { deleted_at: new Date(), is_default: false }
});

// Option 2: Add app_id to composite unique constraint
```

---

## Warnings (Indirect Scoping - Acceptable Pattern)

### 1. SubscriptionService.js - findUnique by id

**Locations:**
- `SubscriptionService.js:63` - cancelSubscription
- `SubscriptionService.js:69` - cancelSubscription (customer lookup)
- `SubscriptionService.js:88-93` - cancelSubscriptionEx (includes customer, verifies app_id)

**Pattern:** Uses `findUnique({ where: { id } })` then verifies `customer.app_id === appId`

**Assessment:** ✓ Acceptable - Verification happens after fetch but before any operations. Not ideal for performance (fetches potentially wrong record) but secure.

**Recommendation (Optional):** Refactor to use findFirst with app_id join for better performance:
```javascript
const subscription = await billingPrisma.billing_subscriptions.findFirst({
  where: {
    id: subscriptionId,
    billing_customers: { app_id: appId }
  },
  include: { billing_customers: true }
});
```

---

### 2. BillingStateService.js - Indirect Queries via billing_customer_id

**Locations:**
- `BillingStateService.js:19-22` - Subscriptions by billing_customer_id
- `BillingStateService.js:30-36` - Payment methods by billing_customer_id (fast-path)
- `BillingStateService.js:41-47` - Payment methods by billing_customer_id (is_default)

**Pattern:** Fetches customer with app_id scope first, then uses billing_customer_id for related queries.

**Assessment:** ✓ Acceptable - Customer is fetched with app_id scope (line 7-12), so billing_customer_id is already verified to belong to app_id. This is transitive scoping.

**Recommendation:** No change needed - pattern is secure and efficient.

---

## Queries Properly Scoped ✓

### CustomerService.js
- ✓ Line 26: getCustomerById - app_id scoped
- ✓ Line 39: findCustomer - app_id scoped

### SubscriptionService.js
- ✓ Line 10: createSubscription - app_id scoped
- ✓ Line 156: changeCycle customer lookup - app_id scoped
- ✓ Line 168: changeCycle subscription lookup - app_id + customer relation verified
- ✓ Line 278: listSubscriptions - app_id scoped via billing_customers relation

### PaymentMethodService.js
- ✓ Line 28: listPaymentMethods - app_id scoped
- ✓ Line 106: setDefaultPaymentMethodById - app_id scoped
- ✓ Line 122-126: updateMany payment methods - app_id scoped
- ✓ Line 163: deletePaymentMethod verification - app_id scoped

### ChargeService.js
- ✓ Line 39: createOneTimeCharge customer lookup - app_id scoped
- ✓ Line 56: existingCharge lookup - app_id scoped
- ✓ Line 75: charge create - app_id scoped
- ✓ Line 101: race condition charge lookup - app_id scoped

### IdempotencyService.js
- ✓ Line 11: getIdempotentResponse - app_id scoped
- ✓ Line 45: storeIdempotentResponse - app_id scoped

### WebhookService.js
- ✓ Line 114: handlePaymentFailure subscription lookup - app_id scoped
- ✓ Line 144: handleSubscriptionUpdate subscription lookup - app_id scoped
- ✓ Line 173: handleSubscriptionCanceled subscription lookup - app_id scoped
- ✓ Line 215: handleRefundEvent charge lookup - app_id scoped
- ✓ Line 306: handleDisputeEvent charge lookup - app_id scoped

---

## Recommended Fixes Priority

### P0 (Critical - Fix Immediately)
1. **WebhookService webhook updates** - Add composite unique constraint on (event_id, app_id)
2. **WebhookService dispute upsert** - Add composite unique constraint on (tilled_dispute_id, app_id)

### P1 (High - Fix Soon)
3. **WebhookService refund queries** - Add app_id to WHERE clauses
4. **PaymentMethodService soft delete** - Use verified record id instead of tilled_payment_method_id

### P2 (Low - Optional Performance)
5. **SubscriptionService indirect lookups** - Refactor to use findFirst with app_id join

---

## Schema Changes Required

```prisma
model billing_webhooks {
  // ... existing fields ...

  @@unique([event_id, app_id], name: "event_id_app_id")
}

model billing_disputes {
  // ... existing fields ...

  @@unique([tilled_dispute_id, app_id], name: "tilled_dispute_id_app_id")
}
```

**Migration Impact:**
- No data changes required
- Index additions only
- Zero downtime deployment possible

---

## Testing Recommendations

After fixes:
1. Add integration test attempting cross-tenant webhook update
2. Add integration test attempting cross-tenant refund access
3. Add integration test attempting cross-tenant dispute manipulation
4. Verify all existing tests still pass

---

## Conclusion

**Summary:**
- 5 critical issues requiring schema + code changes
- 3 acceptable indirect scoping patterns (no changes needed)
- 24 queries properly scoped

**Next Steps:**
1. Create Prisma migration for composite unique constraints
2. Update WebhookService queries to use new constraints
3. Update PaymentMethodService to use verified record id
4. Add security regression tests
5. Document patterns in CONTRIBUTING.md for future development
