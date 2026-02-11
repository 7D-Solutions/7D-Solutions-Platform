# ProrationService Integration Guide

**Phase 3: Proration Engine**
**For:** Developers integrating mid-cycle billing changes

## Table of Contents

1. [Quick Start](#quick-start)
2. [Architecture Overview](#architecture-overview)
3. [Integration Patterns](#integration-patterns)
4. [Common Scenarios](#common-scenarios)
5. [REST API Endpoints](#rest-api-endpoints)
6. [Troubleshooting](#troubleshooting)

---

## Quick Start

### 5-Minute Integration

```javascript
const BillingService = require('./billingService');
const billingService = new BillingService();

// Scenario: Customer upgrades plan mid-cycle
const subscriptionId = 123;
const changeDate = new Date('2026-01-15');
const oldPriceCents = 2500; // $25/month
const newPriceCents = 5000; // $50/month

// Step 1: Calculate proration
const proration = await billingService.calculateProration({
  subscriptionId,
  changeDate,
  oldPriceCents,
  newPriceCents,
  prorationBehavior: 'create_prorations'
});

const netProrationCents = proration.net_change.amount_cents;

// Step 2: Apply discount to prorated amount
const discountResult = await billingService.applyDiscounts(
  'myapp',
  customerId,
  netProrationCents,
  ['UPGRADE10'] // 10% upgrade coupon
);

const afterDiscountCents = discountResult.subtotalAfterDiscount;

// Step 3: Calculate tax on discounted proration
const taxResult = await billingService.calculateTax(
  'myapp',
  customerId,
  afterDiscountCents
);

const finalTotalCents = afterDiscountCents + taxResult.taxAmountCents;

// Step 4: Execute the subscription change with proration
const changeResult = await billingService.applySubscriptionChange(
  subscriptionId,
  {
    newPriceCents,
    oldPriceCents,
    newPlanId: 'plan_premium',
    oldPlanId: 'plan_basic'
  },
  {
    prorationBehavior: 'create_prorations',
    effectiveDate: changeDate
  }
);

console.log(`✅ Upgrade complete. Final charge: $${finalTotalCents / 100}`);
```

---

## Architecture Overview

### How Proration Works

ProrationService calculates mid-cycle billing changes using time-based proration:

```
Billing Period: Jan 1 - Jan 31 (30 days)
Change Date: Jan 15 (mid-cycle)
Days Used: 14 days (Jan 1-14)
Days Remaining: 16 days (Jan 15-31)
Proration Factor: 16/30 = 0.5333

Old Plan: $25/month × 0.5333 = $13.33 credit
New Plan: $50/month × 0.5333 = $26.67 charge
Net Change: $26.67 - $13.33 = $13.34 charge
```

### Integration Points

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Proration     │    │    Discount     │    │      Tax        │
│    Service      │───▶│    Service      │───▶│    Service      │
│  (Phase 3)      │    │   (Phase 2)     │    │   (Phase 1)     │
└─────────────────┘    └─────────────────┘    └─────────────────┘
         │                      │                      │
         ▼                      ▼                      ▼
┌────────────────────────────────────────────────────────────┐
│                 BillingService (Facade)                    │
└────────────────────────────────────────────────────────────┘
```

**Key Principles:**
1. **Order Matters:** Always calculate proration → discount → tax
2. **Discounts apply to net proration amount** (not to full plan prices)
3. **Tax applies to discounted proration amount**
4. **Audit trail** records each step for compliance

---

## Integration Patterns

### Pattern 1: Plan Upgrade with Proration

**Use Case:** Customer upgrades to a higher-tier plan mid-cycle.

```javascript
async function handlePlanUpgrade(subscriptionId, newPlanDetails) {
  // 1. Fetch current subscription
  const subscription = await getSubscription(subscriptionId);

  // 2. Calculate proration
  const proration = await billingService.calculateProration({
    subscriptionId,
    changeDate: new Date(),
    oldPriceCents: subscription.price_cents,
    newPriceCents: newPlanDetails.price_cents,
    prorationBehavior: 'create_prorations'
  });

  // 3. Check if net change is positive (upgrade) or negative (downgrade)
  if (proration.net_change.amount_cents > 0) {
    // Upgrade - customer pays additional amount
    const finalAmount = await applyDiscountAndTax(proration.net_change.amount_cents);

    // 4. Execute change
    const result = await billingService.applySubscriptionChange(
      subscriptionId,
      {
        newPriceCents: newPlanDetails.price_cents,
        oldPriceCents: subscription.price_cents,
        newPlanId: newPlanDetails.planId,
        oldPlanId: subscription.plan_id
      },
      { prorationBehavior: 'create_prorations' }
    );

    // 5. Create invoice with proration charges
    await createInvoice(result.charges, finalAmount);

    return { success: true, charges: result.charges };
  } else {
    // Downgrade - customer receives credit
    return handlePlanDowngrade(subscriptionId, newPlanDetails, proration);
  }
}
```

### Pattern 2: Plan Downgrade with Credit

**Use Case:** Customer downgrades to a lower-tier plan mid-cycle, receiving credit.

```javascript
async function handlePlanDowngrade(subscriptionId, newPlanDetails, proration) {
  // For downgrades, customer receives credit for old plan
  // but still pays prorated amount for new plan

  const chargeableAmount = proration.new_plan.charge_cents;

  if (chargeableAmount > 0) {
    // Apply discount and tax only to new plan charge
    const finalAmount = await applyDiscountAndTax(chargeableAmount);

    const result = await billingService.applySubscriptionChange(
      subscriptionId,
      {
        newPriceCents: newPlanDetails.price_cents,
        oldPriceCents: subscription.price_cents,
        newPlanId: newPlanDetails.planId,
        oldPlanId: subscription.plan_id
      },
      { prorationBehavior: 'create_prorations' }
    );

    // Create invoice with proration credit and charge
    await createInvoice(result.charges, finalAmount);

    return {
      success: true,
      charges: result.charges,
      credit: proration.old_plan.credit_cents
    };
  } else {
    // New plan is free or zero charge
    const result = await billingService.applySubscriptionChange(
      subscriptionId,
      { /* ... */ },
      { prorationBehavior: 'create_prorations' }
    );

    return { success: true, credit: proration.old_plan.credit_cents };
  }
}
```

### Pattern 3: Quantity Changes

**Use Case:** Customer changes quantity of subscribed items mid-cycle.

```javascript
async function handleQuantityChange(subscriptionId, newQuantity) {
  const subscription = await getSubscription(subscriptionId);
  const unitPrice = subscription.price_cents / subscription.quantity;

  const proration = await billingService.calculateProration({
    subscriptionId,
    changeDate: new Date(),
    oldPriceCents: unitPrice,
    newPriceCents: unitPrice, // Same unit price
    oldQuantity: subscription.quantity,
    newQuantity: newQuantity,
    prorationBehavior: 'create_prorations'
  });

  const finalAmount = await applyDiscountAndTax(proration.net_change.amount_cents);

  const result = await billingService.applySubscriptionChange(
    subscriptionId,
    {
      newPriceCents: unitPrice * newQuantity,
      oldPriceCents: subscription.price_cents,
      newQuantity,
      oldQuantity: subscription.quantity
    },
    { prorationBehavior: 'create_prorations' }
  );

  return { success: true, charges: result.charges };
}
```

### Pattern 4: Cancellation with Refund

**Use Case:** Customer cancels subscription mid-cycle, receiving partial refund.

```javascript
async function handleCancellation(subscriptionId, cancellationDate) {
  // 1. Calculate refund
  const refund = await billingService.calculateCancellationRefund(
    subscriptionId,
    cancellationDate,
    'partial_refund'
  );

  if (refund.action === 'refund' && refund.refund_amount_cents > 0) {
    // 2. Process refund through payment processor
    await processRefund(
      subscriptionId,
      refund.refund_amount_cents,
      refund.description
    );

    // 3. Record refund transaction
    await billingService.createRefund('myapp', {
      chargeId: originalChargeId,
      amountCents: refund.refund_amount_cents,
      reason: 'mid_cycle_cancellation',
      metadata: { proration: refund.time_proration }
    });
  }

  // 4. Cancel subscription
  await billingService.cancelSubscription(subscriptionId);

  return { success: true, refund: refund.refund_amount_cents };
}
```

### Pattern 5: Proration Behavior Options

**Three proration behaviors supported:**

```javascript
// 1. 'create_prorations' (default) - Create proration charges/credits
await billingService.applySubscriptionChange(
  subscriptionId,
  changeDetails,
  { prorationBehavior: 'create_prorations' }
);

// 2. 'none' - Update subscription without proration charges
await billingService.applySubscriptionChange(
  subscriptionId,
  changeDetails,
  { prorationBehavior: 'none' }
);

// 3. 'always_invoice' - Create immediate invoice for proration
// (Currently reserved for future implementation)
await billingService.applySubscriptionChange(
  subscriptionId,
  changeDetails,
  { prorationBehavior: 'always_invoice' }
);
```

---

## Common Scenarios

### Scenario 1: SaaS Plan Upgrade

**Context:** A SaaS customer upgrades from Basic ($25/month) to Pro ($50/month) on day 15 of a 30-day billing cycle.

```javascript
const subscription = await getSubscription(saasSubscriptionId);

// Show customer the proration breakdown before confirming
const proration = await billingService.calculateProration({
  subscriptionId: saasSubscriptionId,
  changeDate: new Date(),
  oldPriceCents: 2500,
  newPriceCents: 5000
});

// Display to customer:
console.log(`📊 Proration Breakdown:`);
console.log(`   Days remaining: ${proration.time_proration.daysRemaining}/${proration.time_proration.daysTotal}`);
console.log(`   Credit for unused Basic plan: $${proration.old_plan.credit_cents / 100}`);
console.log(`   Charge for Pro plan (remaining days): $${proration.new_plan.charge_cents / 100}`);
console.log(`   Net change today: $${proration.net_change.amount_cents / 100}`);

// Check for upgrade coupons
const discounts = await billingService.getAvailableDiscounts(
  appId,
  subscription.billing_customer_id,
  { context: 'plan_upgrade' }
);

// Apply best discount
if (discounts.length > 0) {
  const discountResult = await billingService.applyDiscounts(
    appId,
    subscription.billing_customer_id,
    proration.net_change.amount_cents,
    { couponCodes: [discounts[0].code] }
  );

  console.log(`   Discount applied: -$${discountResult.totalDiscountCents / 100}`);
}

// Execute upgrade if customer confirms
```

### Scenario 2: Waste Management Service (TrashTech)

**Context:** Commercial customer adds extra waste containers mid-month.

```javascript
// Each container: $30/month
// Customer has 2 containers, adds 1 more on day 10 of 30-day cycle

const proration = await billingService.calculateProration({
  subscriptionId: trashSubscriptionId,
  changeDate: new Date(),
  oldPriceCents: 3000, // $30 per container
  newPriceCents: 3000, // Same unit price
  oldQuantity: 2,
  newQuantity: 3
});

// Display: "Adding 1 container will add $20.00 to this month's bill"
// (20 days remaining / 30 total days = 0.6667 × $30 = $20)

// Apply commercial volume discount
const discountResult = await billingService.applyDiscounts(
  'trashtech',
  customerId,
  proration.net_change.amount_cents,
  { couponCodes: ['VOLUME10'] } // 10% volume discount
);

// Apply sales tax
const taxResult = await billingService.calculateTax(
  'trashtech',
  customerId,
  discountResult.subtotalAfterDiscount,
  { jurisdictionCode: 'CA' }
);
```

### Scenario 3: Annual to Monthly Conversion

**Context:** Customer switches from annual billing ($120/year) to monthly billing ($12/month) mid-year.

```javascript
// Annual subscription: Jan 1 - Dec 31 (365 days)
// Switch to monthly on July 1 (day 181)

const proration = await billingService.calculateProration({
  subscriptionId: annualSubscriptionId,
  changeDate: new Date('2026-07-01'),
  oldPriceCents: 12000, // $120/year
  newPriceCents: 1200,  // $12/month
  prorationBehavior: 'create_prorations'
});

// Credit for unused annual subscription: $120 × (184/365) ≈ $60.49
// Charge for first month: $12.00
// Net credit: $48.49 (customer receives credit)
```

### Scenario 4: Free Trial Conversion

**Context:** Customer converts from free trial to paid plan mid-cycle.

```javascript
// Free trial: $0/month for first 14 days
// Paid plan: $50/month starting day 15

const proration = await billingService.calculateProration({
  subscriptionId: trialSubscriptionId,
  changeDate: new Date(), // Day 15
  oldPriceCents: 0,       // Free trial
  newPriceCents: 5000,    // $50/month
  prorationBehavior: 'create_prorations'
});

// No credit for free trial
// Charge for remaining 15 days: $50 × (15/30) = $25.00

// Apply welcome discount
const discountResult = await billingService.applyDiscounts(
  appId,
  customerId,
  proration.net_change.amount_cents,
  { couponCodes: ['WELCOME25'] } // 25% off first payment
);
```

---

## REST API Endpoints

ProrationService is accessible through the billing module's REST API:

### Calculate Proration
```
POST /api/billing/v1/proration/calculate
```

**Request Body:**
```json
{
  "subscriptionId": 123,
  "changeDate": "2026-01-15T00:00:00Z",
  "oldPriceCents": 2500,
  "newPriceCents": 5000,
  "oldQuantity": 1,
  "newQuantity": 1,
  "prorationBehavior": "create_prorations"
}
```

**Response:**
```json
{
  "success": true,
  "data": {
    "proration": {
      "subscription_id": 123,
      "change_date": "2026-01-15T00:00:00Z",
      "proration_behavior": "create_prorations",
      "time_proration": {
        "daysUsed": 14,
        "daysRemaining": 16,
        "daysTotal": 30,
        "prorationFactor": 0.5333
      },
      "old_plan": {
        "price_cents": 2500,
        "quantity": 1,
        "total_cents": 2500,
        "credit_cents": 1333
      },
      "new_plan": {
        "price_cents": 5000,
        "quantity": 1,
        "total_cents": 5000,
        "charge_cents": 2667
      },
      "net_change": {
        "amount_cents": 1334,
        "type": "charge",
        "description": "Prorated charge for upgrade"
      }
    }
  }
}
```

### Apply Subscription Change
```
POST /api/billing/v1/subscriptions/:id/change
```

**Request Body:**
```json
{
  "newPriceCents": 5000,
  "oldPriceCents": 2500,
  "newPlanId": "plan_premium",
  "oldPlanId": "plan_basic",
  "options": {
    "prorationBehavior": "create_prorations",
    "effectiveDate": "2026-01-15T00:00:00Z"
  }
}
```

**Response:**
```json
{
  "success": true,
  "data": {
    "subscription": { /* updated subscription */ },
    "proration": { /* proration details */ },
    "charges": [
      {
        "id": 456,
        "charge_type": "proration_credit",
        "amount_cents": -1333,
        "status": "pending"
      },
      {
        "id": 457,
        "charge_type": "proration_charge",
        "amount_cents": 2667,
        "status": "pending"
      }
    ]
  }
}
```

### Calculate Cancellation Refund
```
POST /api/billing/v1/subscriptions/:id/calculate-refund
```

**Request Body:**
```json
{
  "cancellationDate": "2026-01-15T00:00:00Z",
  "refundBehavior": "partial_refund"
}
```

**Response:**
```json
{
  "success": true,
  "data": {
    "refund": {
      "subscription_id": 123,
      "cancellation_date": "2026-01-15T00:00:00Z",
      "refund_behavior": "partial_refund",
      "time_proration": {
        "daysUsed": 14,
        "daysRemaining": 16,
        "daysTotal": 30,
        "prorationFactor": 0.5333
      },
      "total_paid_cents": 5000,
      "refund_amount_cents": 2667,
      "action": "refund",
      "description": "Partial refund of $26.67 for unused service"
    }
  }
}
```

---

## Troubleshooting

### Common Issues

#### Issue 1: Proration calculation returns zero days
**Symptoms:** `daysRemaining` or `daysTotal` is 0.

**Check:**
1. Verify dates are in correct format (JavaScript Date objects)
2. Check timezone normalization (ProrationService uses UTC midnight)
3. Ensure `current_period_start` and `current_period_end` are set on subscription

**Solution:**
```javascript
// Normalize dates before calculation
const normalizedDate = new Date(changeDate);
normalizedDate.setUTCHours(0, 0, 0, 0);
```

#### Issue 2: Negative net change for upgrades
**Symptoms:** Upgrade shows credit instead of charge.

**Check:**
1. Verify `oldPriceCents` and `newPriceCents` are not swapped
2. Check if `newPriceCents` is actually lower than `oldPriceCents`

**Solution:**
```javascript
// Validate prices before calculation
if (newPriceCents < oldPriceCents) {
  console.warn('This appears to be a downgrade, not an upgrade');
}
```

#### Issue 3: Database errors during applySubscriptionChange
**Symptoms:** `applySubscriptionChange` fails with database errors.

**Check:**
1. Verify subscription exists
2. Check database connection
3. Ensure `billing_charges` table has required columns

**Solution:**
```javascript
try {
  await prorationService.applySubscriptionChange(/* ... */);
} catch (error) {
  if (error.code === 'P2002') {
    // Unique constraint violation - charge reference_id already exists
    // Generate new reference_id with timestamp
  }
  if (error.code === 'P2003') {
    // Foreign key constraint - subscription doesn't exist
    // Verify subscription ID
  }
}
```

#### Issue 4: Integration order causing incorrect totals
**Symptoms:** Final total doesn't match expected amount.

**Check:** Verify integration order:
1. **Correct:** Proration → Discount → Tax
2. **Incorrect:** Discount → Proration → Tax or Tax → Discount → Proration

**Solution:**
```javascript
// CORRECT order:
const proration = await calculateProration(...);
const discount = await applyDiscounts(proration.net_change.amount_cents, ...);
const tax = await calculateTax(discount.subtotalAfterDiscount, ...);

// INCORRECT order (don't do this):
const discount = await applyDiscounts(fullPrice, ...);
const proration = await calculateProration(discount.subtotalAfterDiscount, ...);
```

### Debugging Tips

1. **Enable debug logging:**
```javascript
const logger = require('@fireproof/infrastructure/utils/logger');
logger.level = 'debug';
```

2. **Check proration metadata:**
```javascript
const charges = await billingPrisma.billing_charges.findMany({
  where: { charge_type: { in: ['proration_charge', 'proration_credit'] } },
  select: { metadata: true }
});

console.log(JSON.stringify(charges[0].metadata.proration, null, 2));
```

3. **Verify time calculations:**
```javascript
// Manual verification
const daysRemaining = Math.ceil((periodEnd - changeDate) / (24 * 60 * 60 * 1000));
const daysTotal = Math.round((periodEnd - periodStart) / (24 * 60 * 60 * 1000));
const prorationFactor = daysRemaining / daysTotal;
```

### Performance Considerations

1. **Batch operations:** When processing multiple subscription changes, consider batching database operations.
2. **Caching:** Cache tax rates and discount rules to reduce database queries.
3. **Async processing:** For large-scale operations, consider queueing proration calculations.

---

## Support

For issues with ProrationService integration:

1. **Check documentation:** This guide and API reference
2. **Review tests:** `tests/unit/prorationService.test.js`
3. **Examine examples:** `tests/integration/prorationDiscountTaxFlow.test.js`
4. **Contact:** Billing module maintainers

**Version:** 1.0.0
**Last Updated:** 2026-02-04
**Maintainers:** LavenderDog, PearlLynx, MistyBridge