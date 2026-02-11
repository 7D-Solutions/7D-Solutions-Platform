# ProrationService API Reference

**Phase 3: Proration Engine**
**Version:** 1.0.0
**Status:** Production Ready

## Overview

The ProrationService provides comprehensive mid-cycle billing change calculations for subscription-based billing. It handles plan upgrades, downgrades, quantity changes, and cancellations with time-based proration calculations.

**Key Features:**
- Time-based proration calculations (days, months, years)
- Support for plan upgrades, downgrades, and quantity changes
- Multiple proration behaviors (create_prorations, none, always_invoice)
- Refund calculations for cancellations
- Comprehensive audit trail with billing events
- Integration with DiscountService and TaxService
- Generic design (works for any billing scenario)

---

## Table of Contents

1. [Class: ProrationService](#class-prorationservice)
2. [Methods](#methods)
   - [calculateProration()](#calculateproration)
   - [applySubscriptionChange()](#applysubscriptionchange)
   - [calculateCancellationRefund()](#calculatecancellationrefund)
   - [calculateTimeProration()](#calculatetimeproration)
3. [Data Models](#data-models)
4. [Error Handling](#error-handling)
5. [Examples](#examples)
6. [Integration with DiscountService and TaxService](#integration-with-discountservice-and-taxservice)

---

## Class: ProrationService

```javascript
const ProrationService = require('./services/ProrationService');
const prorationService = new ProrationService(getTilledClientFn);
```

### Constructor

```javascript
new ProrationService(getTilledClientFn)
```

**Parameters:**
- `getTilledClientFn` (Function): Function that returns a Tilled client for a given app ID. Required for integration with payment processing.

**Returns:** New ProrationService instance.

**Example:**
```javascript
const ProrationService = require('./services/ProrationService');
const prorationService = new ProrationService((appId) => tilledClients.get(appId));
```

---

## Methods

### calculateProration()

Calculates proration for a mid-cycle subscription change.

```javascript
async calculateProration(params) → Promise<ProrationResult>
```

**Parameters (`params` object):**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `subscriptionId` | number | Yes | Billing subscription ID |
| `changeDate` | Date | Yes | When the change takes effect |
| `newPriceCents` | number | Yes | New plan price in cents |
| `oldPriceCents` | number | Yes | Current plan price in cents |
| `newQuantity` | number | No (default: 1) | New quantity |
| `oldQuantity` | number | No (default: 1) | Current quantity |
| `prorationBehavior` | string | No (default: 'create_prorations') | Proration behavior: 'create_prorations', 'none', or 'always_invoice' |

**Returns:** `Promise<ProrationResult>` - Object containing proration breakdown.

**ProrationResult Structure:**
```javascript
{
  subscription_id: number,
  change_date: Date,
  proration_behavior: string,
  time_proration: {
    daysUsed: number,
    daysRemaining: number,
    daysTotal: number,
    prorationFactor: number,
    note?: string
  },
  old_plan: {
    price_cents: number,
    quantity: number,
    total_cents: number,
    credit_cents: number
  },
  new_plan: {
    price_cents: number,
    quantity: number,
    total_cents: number,
    charge_cents: number
  },
  net_change: {
    amount_cents: number,
    type: 'charge' | 'credit',
    description: string
  }
}
```

**Example:**
```javascript
const proration = await prorationService.calculateProration({
  subscriptionId: 123,
  changeDate: new Date('2026-01-15'),
  oldPriceCents: 2500, // $25.00
  newPriceCents: 5000, // $50.00
  oldQuantity: 1,
  newQuantity: 1,
  prorationBehavior: 'create_prorations'
});

console.log(`Net change: $${proration.net_change.amount_cents / 100}`);
console.log(`Type: ${proration.net_change.type}`);
```

**Throws:**
- `ValidationError` - Missing required parameters or invalid values
- `NotFoundError` - Subscription not found

---

### applySubscriptionChange()

Executes mid-cycle subscription change with proration, creating proration charges/credits and updating the subscription.

```javascript
async applySubscriptionChange(subscriptionId, changeDetails, options) → Promise<ChangeResult>
```

**Parameters:**
- `subscriptionId` (number): Billing subscription ID
- `changeDetails` (object): Change details
  - `newPriceCents` (number): New plan price in cents
  - `oldPriceCents` (number): Current plan price in cents
  - `newQuantity` (number, optional): New quantity (default: 1)
  - `oldQuantity` (number, optional): Current quantity (default: 1)
  - `newPlanId` (string, optional): New plan identifier
  - `oldPlanId` (string, optional): Current plan identifier
- `options` (object, optional): Proration options
  - `prorationBehavior` (string): 'create_prorations', 'none', or 'always_invoice' (default: 'create_prorations')
  - `effectiveDate` (Date): When change takes effect (default: current date)
  - `invoiceImmediately` (boolean): Whether to invoice immediately (default: false)

**Returns:** `Promise<ChangeResult>` - Object containing updated subscription, proration details, and created charges.

**ChangeResult Structure:**
```javascript
{
  subscription: Subscription, // Updated subscription record
  proration: ProrationResult, // Proration calculation (null if behavior is 'none')
  charges: Array<Charge> // Created proration charges/credits
}
```

**Example:**
```javascript
const result = await prorationService.applySubscriptionChange(
  123,
  {
    newPriceCents: 5000,
    oldPriceCents: 2500,
    newPlanId: 'plan_premium',
    oldPlanId: 'plan_basic'
  },
  {
    prorationBehavior: 'create_prorations',
    effectiveDate: new Date('2026-01-15'),
    invoiceImmediately: false
  }
);

console.log(`Created ${result.charges.length} charges`);
console.log(`Subscription updated to $${result.subscription.price_cents / 100}/month`);
```

**Throws:**
- `ValidationError` - Missing required parameters
- `NotFoundError` - Subscription not found

---

### calculateCancellationRefund()

Calculates refund amount for subscription cancellation.

```javascript
async calculateCancellationRefund(subscriptionId, cancellationDate, refundBehavior) → Promise<RefundResult>
```

**Parameters:**
- `subscriptionId` (number): Billing subscription ID
- `cancellationDate` (Date): When cancellation takes effect
- `refundBehavior` (string): Refund behavior: 'partial_refund', 'account_credit', or 'none' (default: 'partial_refund')

**Returns:** `Promise<RefundResult>` - Object containing refund calculation.

**RefundResult Structure:**
```javascript
{
  subscription_id: number,
  cancellation_date: Date,
  refund_behavior: string,
  time_proration: {
    daysUsed: number,
    daysRemaining: number,
    daysTotal: number,
    prorationFactor: number
  },
  total_paid_cents: number,
  refund_amount_cents: number,
  action: 'refund' | 'account_credit' | 'none',
  description: string
}
```

**Example:**
```javascript
const refund = await prorationService.calculateCancellationRefund(
  123,
  new Date('2026-01-15'),
  'partial_refund'
);

console.log(`Refund amount: $${refund.refund_amount_cents / 100}`);
console.log(`Action: ${refund.action}`);
```

**Throws:**
- `ValidationError` - Missing required parameters
- `NotFoundError` - Subscription not found

---

### calculateTimeProration()

Calculates time-based proration factor for a given change date within a billing period. This is a utility method that can be used independently.

```javascript
calculateTimeProration(changeDate, periodEnd, periodStart) → TimeProration
```

**Parameters:**
- `changeDate` (Date): When change takes effect
- `periodEnd` (Date): Billing period end date
- `periodStart` (Date): Billing period start date

**Returns:** `TimeProration` - Object containing days used, remaining, total, and proration factor.

**TimeProration Structure:**
```javascript
{
  daysUsed: number,
  daysRemaining: number,
  daysTotal: number,
  prorationFactor: number,
  note?: string // 'change_at_period_start' or 'change_at_period_end' for edge cases
}
```

**Example:**
```javascript
const periodStart = new Date('2026-01-01');
const periodEnd = new Date('2026-01-31');
const changeDate = new Date('2026-01-15');

const timeProration = prorationService.calculateTimeProration(
  changeDate,
  periodEnd,
  periodStart
);

console.log(`Days remaining: ${timeProration.daysRemaining}/${timeProration.daysTotal}`);
console.log(`Proration factor: ${timeProration.prorationFactor}`);
```

---

## Data Models

### Subscription Record
Reference to the `billing_subscriptions` table structure used by ProrationService.

**Fields used:**
- `id` (number): Subscription ID
- `billing_customer_id` (number): Customer ID
- `price_cents` (number): Current price in cents
- `current_period_start` (Date): Billing period start
- `current_period_end` (Date): Billing period end
- `metadata` (JSON): Subscription metadata

### Proration Charge
Proration charges are stored in `billing_charges` table with `charge_type` of 'proration_charge' or 'proration_credit'.

**Fields:**
- `charge_type`: 'proration_charge' (positive amount) or 'proration_credit' (negative amount)
- `amount_cents`: Prorated amount in cents
- `reason`: 'mid_cycle_upgrade' or 'mid_cycle_downgrade'
- `reference_id`: Unique reference ID format: `proration_sub_{subscriptionId}_{date}_{type}`
- `metadata.proration`: Detailed proration breakdown

### Proration Event
Audit events are stored in `billing_events` table with `event_type` 'proration_applied'.

**Fields:**
- `event_type`: 'proration_applied'
- `source`: 'proration_service'
- `entity_type`: 'subscription'
- `entity_id`: Subscription ID
- `payload`: Complete proration details

---

## Error Handling

ProrationService uses the shared error classes from `utils/errors`:

### ValidationError
Thrown when input parameters are invalid or missing.

**Example:**
```javascript
try {
  await prorationService.calculateProration({ /* missing required fields */ });
} catch (error) {
  if (error.name === 'ValidationError') {
    console.error('Validation error:', error.message);
  }
}
```

### NotFoundError
Thrown when referenced subscription is not found.

**Example:**
```javascript
try {
  await prorationService.calculateProration({
    subscriptionId: 99999, // Non-existent
    changeDate: new Date(),
    oldPriceCents: 1000,
    newPriceCents: 2000
  });
} catch (error) {
  if (error.name === 'NotFoundError') {
    console.error('Subscription not found:', error.message);
  }
}
```

---

## Examples

### Basic Proration Calculation
```javascript
const proration = await prorationService.calculateProration({
  subscriptionId: 123,
  changeDate: new Date('2026-01-15'),
  oldPriceCents: 2500, // $25/month
  newPriceCents: 5000, // $50/month
  prorationBehavior: 'create_prorations'
});

console.log('Proration factor:', proration.time_proration.prorationFactor);
console.log('Old plan credit:', proration.old_plan.credit_cents);
console.log('New plan charge:', proration.new_plan.charge_cents);
console.log('Net change:', proration.net_change.amount_cents);
```

### Plan Upgrade with Proration
```javascript
const result = await prorationService.applySubscriptionChange(
  123,
  {
    newPriceCents: 5000,
    oldPriceCents: 2500,
    newPlanId: 'plan_premium',
    oldPlanId: 'plan_basic'
  },
  {
    prorationBehavior: 'create_prorations',
    effectiveDate: new Date('2026-01-15')
  }
);

// Create invoice with proration charges
for (const charge of result.charges) {
  await createInvoiceLineItem({
    chargeId: charge.id,
    amount: charge.amount_cents,
    description: charge.metadata.proration.description
  });
}
```

### Cancellation Refund
```javascript
const refund = await prorationService.calculateCancellationRefund(
  123,
  new Date('2026-01-15'),
  'partial_refund'
);

if (refund.action === 'refund') {
  await processRefund(refund.refund_amount_cents, refund.description);
}
```

---

## Integration with DiscountService and TaxService

### Discount → Proration → Tax Flow
ProrationService is designed to integrate seamlessly with DiscountService (Phase 2) and TaxService (Phase 1) in the following order:

1. **Calculate proration** - Determine net change amount
2. **Apply discounts** - Apply coupons to net proration amount
3. **Calculate tax** - Calculate tax on discounted amount

**Example:**
```javascript
// 1. Calculate proration (Phase 3)
const proration = await prorationService.calculateProration({
  subscriptionId: 123,
  changeDate: new Date('2026-01-15'),
  oldPriceCents: 2500,
  newPriceCents: 5000
});

const netProrationCents = proration.net_change.amount_cents;

// 2. Apply discount (Phase 2)
const discountResult = await discountService.calculateDiscounts(
  appId,
  customerId,
  netProrationCents,
  { couponCodes: ['UPGRADE10'] }
);

const afterDiscountCents = discountResult.subtotalAfterDiscount;

// 3. Calculate tax (Phase 1)
const taxResult = await taxService.calculateTax(
  appId,
  customerId,
  afterDiscountCents
);

const finalTotalCents = afterDiscountCents + taxResult.taxAmountCents;
console.log(`Final total: $${finalTotalCents / 100}`);
```

### BillingService Integration
ProrationService is integrated into BillingService as a facade method:

```javascript
// Through BillingService
const billingService = new BillingService();

// Proration methods are available
const proration = await billingService.calculateProration(params);
const changeResult = await billingService.applySubscriptionChange(subscriptionId, changeDetails, options);
const refund = await billingService.calculateCancellationRefund(subscriptionId, cancellationDate, refundBehavior);
```

---

## Testing

### Unit Tests
Comprehensive unit tests are available in `tests/unit/prorationService.test.js`.

**Run tests:**
```bash
cd packages/billing
npm test -- prorationService.test.js
```

### Integration Tests
Integration tests with DiscountService and TaxService are available in `tests/integration/prorationDiscountTaxFlow.test.js`.

**Run integration tests:**
```bash
cd packages/billing
npm test -- prorationDiscountTaxFlow.test.js
```

---

**Version History:**
- 1.0.0 (2026-02-04): Initial release with Phase 3 implementation

**Maintainers:** LavenderDog (implementation), PearlLynx (plan), MistyBridge (coordination)