# DiscountService API Reference

**Phase 2: Discount & Promotion Engine**
**Version:** 1.0.0
**Status:** Production Ready

## Overview

The DiscountService provides comprehensive discount calculation and management functionality for the billing module. It supports multiple discount types, coupon validation, stacking rules, and detailed audit trails.

**Key Features:**
- Multiple discount types (percentage, fixed, volume, seasonal, referral, contract)
- Coupon code validation and eligibility checking
- Stacking rules with priority system
- Automatic volume discount detection
- Comprehensive audit trail
- App-scoped security
- Generic design (works for any billing scenario)

---

## Table of Contents

1. [Class: DiscountService](#class-discountservice)
2. [Methods](#methods)
   - [calculateDiscounts()](#calculatediscounts)
   - [validateCoupon()](#validatecoupon)
   - [recordDiscount()](#recorddiscount)
   - [getDiscountsForInvoice()](#getdiscountsforinvoice)
   - [getAvailableDiscounts()](#getavailablediscounts)
3. [Data Models](#data-models)
4. [Discount Types](#discount-types)
5. [Stacking Rules](#stacking-rules)
6. [Error Handling](#error-handling)
7. [Examples](#examples)

---

## Class: DiscountService

```javascript
const DiscountService = require('./services/DiscountService');
const discountService = new DiscountService();
```

### Constructor

```javascript
new DiscountService()
```

No parameters required. The service uses the global `billingPrisma` client for database operations.

---

## Methods

### calculateDiscounts()

Calculate applicable discounts for a billing scenario.

**Signature:**
```javascript
async calculateDiscounts(appId, customerId, subtotalCents, options = {})
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `customerId` (number, required) - Customer ID
- `subtotalCents` (number, required) - Pre-discount amount in cents
- `options` (object, optional)
  - `couponCodes` (string[]) - User-provided coupon codes
  - `category` (string) - Customer category (e.g., 'residential', 'commercial')
  - `products` (Array<{type, quantity}>) - Products being purchased
  - `metadata` (object) - Additional context for industry-specific logic

**Returns:** Promise<Object>
```javascript
{
  discounts: [                        // Applied discounts
    {
      couponId: 1,
      code: 'SUMMER20',
      type: 'percentage',
      amountCents: 2000,              // $20.00 discount
      description: '20% off',
      stackable: false,
      priority: 10,
      metadata: {
        originalValue: 20,
        couponType: 'percentage'
      }
    }
  ],
  totalDiscountCents: 2000,           // Total discount
  subtotalBeforeDiscount: 10000,      // Original subtotal
  subtotalAfterDiscount: 8000,        // After discounts
  appliedInOrder: ['SUMMER20'],       // Discount codes in application order
  rejectedCoupons: [                  // Rejected codes with reasons
    { code: 'EXPIRED10', reason: 'Coupon has expired' }
  ]
}
```

**Throws:**
- `ValidationError` - Missing required parameters or invalid amount
- `NotFoundError` - Customer not found

**Example:**
```javascript
const discountResult = await discountService.calculateDiscounts(
  'myapp',
  123,
  10000,  // $100.00
  {
    couponCodes: ['SUMMER20', 'LOYALTY5'],
    category: 'residential',
    products: [
      { type: 'standard', quantity: 3 },
      { type: 'premium', quantity: 1 }
    ]
  }
);

console.log(`Discount: $${discountResult.totalDiscountCents / 100}`);
// Output: Discount: $20.00
```

---

### validateCoupon()

Validate a coupon code without applying it.

**Signature:**
```javascript
async validateCoupon(appId, couponCode, context = {})
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `couponCode` (string, required) - Coupon code to validate
- `context` (object, optional)
  - `category` (string) - Customer category
  - `productTypes` (string[]) - Product types in cart
  - `totalQuantity` (number) - Total quantity

**Returns:** Promise<Object>
```javascript
// Valid coupon
{
  valid: true,
  coupon: {
    id: 1,
    code: 'SUMMER20',
    type: 'percentage',
    value: 20,
    description: '20% off'
  }
}

// Invalid coupon
{
  valid: false,
  reason: 'Coupon has expired'
}
```

**Throws:**
- `ValidationError` - Missing required parameters

**Example:**
```javascript
const validation = await discountService.validateCoupon(
  'myapp',
  'SUMMER20',
  { category: 'residential' }
);

if (validation.valid) {
  console.log(`Valid: ${validation.coupon.description}`);
} else {
  console.log(`Invalid: ${validation.reason}`);
}
```

---

### recordDiscount()

Record a discount application for audit trail.

**Signature:**
```javascript
async recordDiscount(appId, discountDetails)
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `discountDetails` (object, required)
  - `invoiceId` (number, optional) - Invoice ID
  - `chargeId` (number, optional) - Charge ID
  - `couponId` (number, optional) - Coupon ID
  - `customerId` (number, optional) - Customer ID
  - `discountType` (string) - Type of discount ('percentage', 'fixed', 'volume', etc.)
  - `discountAmountCents` (number, required) - Discount amount in cents
  - `description` (string) - Human-readable description
  - `quantity` (number, optional) - Quantity involved
  - `category` (string, optional) - Customer category
  - `productTypes` (array, optional) - Product types discounted
  - `metadata` (object, optional) - Additional metadata
  - `createdBy` (string, optional) - Creator identifier

**Returns:** Promise<Object> - Created discount application record

**Throws:**
- `ValidationError` - Missing required fields

**Example:**
```javascript
const discountRecord = await discountService.recordDiscount(
  'myapp',
  {
    invoiceId: 456,
    couponId: 1,
    customerId: 123,
    discountType: 'percentage',
    discountAmountCents: 2000,
    description: '20% off Summer Sale',
    metadata: { campaign: 'summer_2025' }
  }
);

console.log(`Recorded discount: ${discountRecord.id}`);
```

---

### getDiscountsForInvoice()

Get all discount applications for an invoice.

**Signature:**
```javascript
async getDiscountsForInvoice(appId, invoiceId)
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `invoiceId` (number, required) - Invoice ID

**Returns:** Promise<Array<Object>> - Discount applications with coupon details

**Throws:**
- `ValidationError` - Missing parameters

**Example:**
```javascript
const discounts = await discountService.getDiscountsForInvoice('myapp', 456);

const totalDiscount = discounts.reduce((sum, d) => {
  return sum + d.discount_amount_cents;
}, 0);

console.log(`Total discounts on invoice: $${totalDiscount / 100}`);
```

---

### getAvailableDiscounts()

Get available discounts/promotions for a customer.

**Signature:**
```javascript
async getAvailableDiscounts(appId, customerId, context = {})
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `customerId` (number, optional) - Customer ID
- `context` (object, optional)
  - `category` (string) - Customer category for filtering

**Returns:** Promise<Array<Object>>
```javascript
[
  {
    id: 1,
    code: 'SUMMER20',
    type: 'percentage',
    value: 20,
    description: '20% off',
    stackable: false,
    seasonalStart: '2025-06-01T00:00:00.000Z',
    seasonalEnd: '2025-08-31T23:59:59.000Z',
    minQuantity: null,
    productCategories: ['standard', 'premium'],
    customerSegments: ['residential', 'commercial']
  }
]
```

**Throws:**
- `ValidationError` - Missing appId

**Example:**
```javascript
const available = await discountService.getAvailableDiscounts(
  'myapp',
  123,
  { category: 'residential' }
);

console.log(`${available.length} discounts available`);
available.forEach(d => console.log(`  - ${d.code}: ${d.description}`));
```

---

## Data Models

### Coupon

```javascript
{
  id: 1,
  app_id: 'myapp',
  code: 'SUMMER20',
  coupon_type: 'percentage',           // 'percentage', 'fixed', 'volume', 'referral', 'contract'
  value: 20,                           // 20% for percentage, cents for fixed
  active: true,
  redeem_by: '2025-12-31T23:59:59.000Z',
  max_redemptions: 1000,

  // Phase 2 fields
  product_categories: ['standard', 'premium'],  // Product types this applies to
  customer_segments: ['residential'],           // Customer segments eligible
  min_quantity: 2,                              // Minimum quantity required
  max_discount_amount_cents: 5000,              // Cap at $50.00
  seasonal_start_date: '2025-06-01T00:00:00.000Z',
  seasonal_end_date: '2025-08-31T23:59:59.000Z',
  volume_tiers: [                               // For volume discounts
    { min: 5, max: 9, discount: 5 },
    { min: 10, max: 19, discount: 10 },
    { min: 20, max: null, discount: 15 }
  ],
  referral_tier: 'standard',                    // For referral discounts
  contract_term_months: 12,                     // For contract discounts
  stackable: false,                             // Can combine with others
  priority: 10,                                 // Higher = applied first

  metadata: {},
  created_at: '2025-01-01T00:00:00.000Z',
  updated_at: '2025-01-01T00:00:00.000Z'
}
```

### Discount Application

```javascript
{
  id: 1,
  app_id: 'myapp',
  invoice_id: 456,
  charge_id: null,
  coupon_id: 1,
  customer_id: 123,
  discount_type: 'percentage',
  discount_amount_cents: 2000,         // $20.00
  description: '20% off Summer Sale',
  quantity: 4,
  category: 'residential',
  product_types: ['standard'],
  metadata: { campaign: 'summer_2025' },
  applied_at: '2025-02-04T...',
  created_by: 'system'
}
```

---

## Discount Types

### Percentage

Percentage off the subtotal.

```javascript
{
  coupon_type: 'percentage',
  value: 20  // 20% off
}
```

**Calculation:** `subtotalCents * (value / 100)`

### Fixed

Fixed dollar amount off.

```javascript
{
  coupon_type: 'fixed',
  value: 500  // $5.00 off (in cents)
}
```

**Calculation:** `value` (directly subtracted)

### Volume (Tiered)

Discount based on quantity purchased.

```javascript
{
  coupon_type: 'volume',
  volume_tiers: [
    { min: 5, max: 9, discount: 5 },     // 5-9 units: 5% off
    { min: 10, max: 19, discount: 10 },  // 10-19 units: 10% off
    { min: 20, max: null, discount: 15 } // 20+ units: 15% off
  ]
}
```

**Calculation:** Customer qualifies for highest tier where `quantity >= min`

### Seasonal

Time-limited promotions.

```javascript
{
  coupon_type: 'percentage',
  value: 25,
  seasonal_start_date: '2025-06-01',
  seasonal_end_date: '2025-08-31'
}
```

**Validation:** Only valid between start and end dates

### Referral

Discount for referrals.

```javascript
{
  coupon_type: 'referral',
  value: 1000,  // $10.00 credit
  referral_tier: 'standard'
}
```

### Contract Term

Discount for longer commitments.

```javascript
{
  coupon_type: 'contract',
  value: 10,  // 10% off
  contract_term_months: 12
}
```

---

## Stacking Rules

### Priority System

Discounts are sorted by `priority` (higher first).

```javascript
// Higher priority discounts are evaluated first
{ code: 'VIP50', priority: 20 }     // Evaluated 1st
{ code: 'SUMMER20', priority: 10 }  // Evaluated 2nd
{ code: 'LOYALTY5', priority: 5 }   // Evaluated 3rd
```

### Non-Stackable Discounts

When `stackable: false`:
- Only one non-stackable discount can apply
- If multiple non-stackable discounts exist at same priority, highest amount wins
- Non-stackable discounts are applied first

### Stackable Discounts

When `stackable: true`:
- Can combine with other discounts
- Applied after non-stackable discount
- Percentage discounts recalculated on remaining amount

### Example

```javascript
// Discounts:
// 1. VIP50: 50% off, non-stackable, priority: 20
// 2. SUMMER20: 20% off, non-stackable, priority: 10
// 3. LOYALTY5: 5% off, stackable, priority: 5

// Subtotal: $100.00

// Result:
// - VIP50 wins (highest priority non-stackable): $50.00 off
// - LOYALTY5 stacks: 5% of remaining $50 = $2.50 off
// - SUMMER20 rejected (non-stackable, lower priority)
// - Total discount: $52.50
```

---

## Error Handling

### Error Types

**ValidationError**
- Missing required parameters
- Invalid data types or formats
- Business rule violations

**NotFoundError**
- Customer not found
- Coupon not found

### Error Handling Example

```javascript
try {
  const discountResult = await discountService.calculateDiscounts(
    appId, customerId, amount, options
  );
} catch (error) {
  if (error instanceof ValidationError) {
    console.error('Validation error:', error.message);
    // Handle validation error (400 Bad Request)
  } else if (error instanceof NotFoundError) {
    console.error('Not found:', error.message);
    // Handle not found error (404 Not Found)
  } else {
    console.error('Unexpected error:', error);
    // Handle other errors (500 Internal Server Error)
  }
}
```

---

## Examples

### Complete Invoice Generation with Discounts and Tax

```javascript
// 1. Calculate subtotal from line items
const subtotalCents = 10000; // $100.00

// 2. Apply discounts
const discountResult = await discountService.calculateDiscounts(
  appId,
  customerId,
  subtotalCents,
  { couponCodes: ['SUMMER20'] }
);

// 3. Calculate tax on discounted amount (discount-before-tax)
const taxResult = await taxService.calculateTax(
  appId,
  customerId,
  discountResult.subtotalAfterDiscount
);

// 4. Record discount applications
for (const discount of discountResult.discounts) {
  await discountService.recordDiscount(appId, {
    invoiceId: newInvoice.id,
    couponId: discount.couponId,
    customerId,
    discountType: discount.type,
    discountAmountCents: discount.amountCents,
    description: discount.description
  });
}

// 5. Record tax calculations
for (const taxItem of taxResult.breakdown) {
  await taxService.recordTaxCalculation(
    appId,
    taxItem.taxRateId,
    taxItem.taxableAmountCents,
    taxItem.taxAmountCents,
    { invoiceId: newInvoice.id }
  );
}

// 6. Create final invoice
const invoice = {
  subtotal: subtotalCents,
  discount: discountResult.totalDiscountCents,
  tax: taxResult.taxAmountCents,
  total: discountResult.subtotalAfterDiscount + taxResult.taxAmountCents
};
```

### Setting Up Coupons

```javascript
// Percentage discount
await billingPrisma.billing_coupons.create({
  data: {
    app_id: 'myapp',
    code: 'WELCOME10',
    coupon_type: 'percentage',
    value: 10,
    active: true,
    customer_segments: ['new'],
    max_redemptions: 1000
  }
});

// Volume discount
await billingPrisma.billing_coupons.create({
  data: {
    app_id: 'myapp',
    code: 'BULK_DISCOUNT',
    coupon_type: 'volume',
    value: 0,  // Calculated from tiers
    active: true,
    volume_tiers: [
      { min: 10, max: 24, discount: 5 },
      { min: 25, max: 49, discount: 10 },
      { min: 50, max: null, discount: 15 }
    ],
    stackable: true,
    priority: -1  // Apply after other discounts
  }
});

// Seasonal promotion
await billingPrisma.billing_coupons.create({
  data: {
    app_id: 'myapp',
    code: 'HOLIDAY25',
    coupon_type: 'percentage',
    value: 25,
    active: true,
    seasonal_start_date: new Date('2025-12-20'),
    seasonal_end_date: new Date('2025-12-31'),
    max_discount_amount_cents: 10000  // Cap at $100
  }
});
```

### Validating Coupon at Checkout

```javascript
async function handleCouponEntry(appId, couponCode, cart) {
  const validation = await discountService.validateCoupon(
    appId,
    couponCode,
    {
      category: cart.customer.category,
      productTypes: cart.items.map(i => i.type),
      totalQuantity: cart.items.reduce((sum, i) => sum + i.quantity, 0)
    }
  );

  if (!validation.valid) {
    return {
      success: false,
      message: validation.reason
    };
  }

  return {
    success: true,
    message: `Coupon applied: ${validation.coupon.description}`,
    coupon: validation.coupon
  };
}
```

---

## Integration with BillingService

DiscountService is integrated into the BillingService facade:

```javascript
const BillingService = require('./billingService');
const billingService = new BillingService();

// All DiscountService methods are available via delegation:
await billingService.calculateDiscounts(appId, customerId, amount, options);
await billingService.applyDiscounts(appId, customerId, amount, couponCodes);
await billingService.validateCoupon(appId, couponCode, context);
await billingService.recordDiscount(appId, discountDetails);
await billingService.getDiscountsForInvoice(appId, invoiceId);
await billingService.getAvailableDiscounts(appId, customerId, context);
```

---

## Best Practices

1. **Always apply discounts BEFORE tax**
   - Calculate discounts first, then calculate tax on discounted amount
   - This is the standard "discount-before-tax" flow

2. **Record discount applications for audit trail**
   - Always call `recordDiscount()` after applying discounts
   - Links discount applications to invoices/charges for compliance

3. **Use appropriate discount types**
   - `percentage` for percentage-based discounts
   - `fixed` for flat dollar amounts
   - `volume` for quantity-based tiering

4. **Set up stacking rules carefully**
   - Use `stackable: false` for major promotions
   - Use `stackable: true` for loyalty/volume discounts
   - Higher `priority` = applied first

5. **Use max_discount_amount_cents for caps**
   ```javascript
   {
     coupon_type: 'percentage',
     value: 50,  // 50% off
     max_discount_amount_cents: 5000  // But max $50
   }
   ```

6. **Validate coupons before checkout**
   - Use `validateCoupon()` to check eligibility
   - Show users why coupons were rejected

---

## Testing

See `packages/billing/tests/unit/discountService.test.js` for comprehensive test examples.

**Test Coverage:**
- 45+ test cases
- All methods tested
- All discount types tested
- Edge cases and error scenarios
- Stacking rules validation
- Integration scenarios

**Run Tests:**
```bash
npm test -- discountService.test.js
```

---

## Support

For questions or issues:
- See integration examples in this document
- Check test cases for usage patterns
- Review error handling section for common issues

---

**Last Updated:** 2025-02-04
**Phase 2 Status:** Production Ready
