# DiscountService Integration Guide

**Phase 2: Discount & Promotion Engine**
**For:** Developers integrating discounts into billing flows

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

// Step 1: Calculate discounts during invoice generation
const discountResult = await billingService.calculateDiscounts(
  'myapp',
  customerId,
  10000,  // $100.00 subtotal
  { couponCodes: ['SUMMER20'] }
);

// Step 2: Calculate tax on DISCOUNTED amount
const taxResult = await billingService.calculateTax(
  'myapp',
  customerId,
  discountResult.subtotalAfterDiscount
);

// Step 3: Record discount for audit trail
for (const discount of discountResult.discounts) {
  await billingService.recordDiscount('myapp', {
    invoiceId: newInvoice.id,
    couponId: discount.couponId,
    customerId,
    discountType: discount.type,
    discountAmountCents: discount.amountCents,
    description: discount.description
  });
}

// Step 4: Final totals
const totalCents = discountResult.subtotalAfterDiscount + taxResult.taxAmountCents;
```

---

## Architecture Overview

### Component Diagram

```
┌─────────────────────────────────────────────────────────┐
│                   BillingService                        │
│                     (Facade)                            │
└─────────────┬───────────────────────────────────────────┘
              │
              ├─> CustomerService
              ├─> PaymentMethodService
              ├─> SubscriptionService
              ├─> ChargeService
              ├─> TaxService (Phase 1)
              └─> DiscountService (Phase 2) ← YOU ARE HERE
                      │
                      ├─> calculateDiscounts()
                      ├─> validateCoupon()
                      ├─> recordDiscount()
                      ├─> getDiscountsForInvoice()
                      └─> getAvailableDiscounts()
```

### Data Flow (Discount-Before-Tax)

```
Customer Order
    ↓
Calculate Subtotal
    ↓
Apply Discounts ← DiscountService.calculateDiscounts()
    ↓
Calculate Tax on Discounted Amount ← TaxService.calculateTax()
    ↓
Record Discount ← DiscountService.recordDiscount()
    ↓
Record Tax ← TaxService.recordTaxCalculation()
    ↓
Generate Invoice
```

### Database Schema

```
billing_coupons
├── id (PK)
├── app_id
├── code
├── coupon_type (percentage, fixed, volume, referral, contract)
├── value
├── active
├── redeem_by
├── max_redemptions
├── product_categories (JSON)
├── customer_segments (JSON)
├── min_quantity
├── max_discount_amount_cents
├── seasonal_start_date
├── seasonal_end_date
├── volume_tiers (JSON)
├── referral_tier
├── contract_term_months
├── stackable
└── priority

billing_discount_applications
├── id (PK)
├── app_id
├── invoice_id (FK)
├── charge_id (FK)
├── coupon_id (FK)
├── customer_id
├── discount_type
├── discount_amount_cents
├── description
├── quantity
├── category
├── product_types (JSON)
├── metadata (JSON)
├── applied_at
└── created_by
```

---

## Integration Patterns

### Pattern 1: Invoice with Discounts and Tax

**Use Case:** Standard invoice with coupon codes

```javascript
async function generateInvoice(appId, customerId, lineItems, couponCodes = []) {
  // 1. Calculate subtotal
  const subtotalCents = lineItems.reduce((sum, item) => sum + item.priceCents, 0);

  // 2. Apply discounts
  const discountResult = await billingService.calculateDiscounts(
    appId,
    customerId,
    subtotalCents,
    {
      couponCodes,
      products: lineItems.map(item => ({
        type: item.productType,
        quantity: item.quantity
      }))
    }
  );

  // 3. Calculate tax on discounted amount
  const taxResult = await billingService.calculateTax(
    appId,
    customerId,
    discountResult.subtotalAfterDiscount
  );

  // 4. Create invoice
  const invoice = await createInvoice(appId, customerId, {
    subtotalCents,
    discountCents: discountResult.totalDiscountCents,
    taxCents: taxResult.taxAmountCents,
    totalCents: discountResult.subtotalAfterDiscount + taxResult.taxAmountCents
  });

  // 5. Record discounts
  for (const discount of discountResult.discounts) {
    await billingService.recordDiscount(appId, {
      invoiceId: invoice.id,
      couponId: discount.couponId,
      customerId,
      discountType: discount.type,
      discountAmountCents: discount.amountCents,
      description: discount.description
    });
  }

  // 6. Record tax
  for (const taxItem of taxResult.breakdown) {
    await billingService.recordTaxCalculation(
      appId,
      taxItem.taxRateId,
      taxItem.taxableAmountCents,
      taxItem.taxAmountCents,
      { invoiceId: invoice.id }
    );
  }

  return {
    invoice,
    discountResult,
    taxResult
  };
}
```

### Pattern 2: Coupon Validation at Checkout

**Use Case:** Validate coupon before applying

```javascript
async function applyCouponAtCheckout(appId, couponCode, cart) {
  // Build context from cart
  const context = {
    category: cart.customer.category,
    productTypes: cart.items.map(i => i.type),
    totalQuantity: cart.items.reduce((sum, i) => sum + i.quantity, 0)
  };

  // Validate coupon
  const validation = await billingService.validateCoupon(
    appId,
    couponCode,
    context
  );

  if (!validation.valid) {
    return {
      success: false,
      error: validation.reason
    };
  }

  // Preview discount
  const preview = await billingService.calculateDiscounts(
    appId,
    cart.customer.id,
    cart.subtotalCents,
    { couponCodes: [couponCode], ...context }
  );

  return {
    success: true,
    coupon: validation.coupon,
    discountPreview: {
      amount: preview.totalDiscountCents,
      newSubtotal: preview.subtotalAfterDiscount
    }
  };
}
```

### Pattern 3: Automatic Volume Discounts

**Use Case:** Apply volume discounts without coupon codes

```javascript
async function calculateOrderWithVolumeDiscounts(appId, customerId, lineItems) {
  const subtotalCents = lineItems.reduce((sum, item) => sum + item.priceCents, 0);
  const totalQuantity = lineItems.reduce((sum, item) => sum + item.quantity, 0);

  // calculateDiscounts automatically finds volume discounts
  const discountResult = await billingService.calculateDiscounts(
    appId,
    customerId,
    subtotalCents,
    {
      couponCodes: [],  // No explicit coupons
      products: lineItems.map(item => ({
        type: item.productType,
        quantity: item.quantity
      }))
    }
  );

  // Volume discount is automatically detected and applied
  const volumeDiscount = discountResult.discounts.find(d => d.type === 'volume');

  if (volumeDiscount) {
    console.log(`Volume discount applied: ${volumeDiscount.description}`);
  }

  return discountResult;
}
```

### Pattern 4: Displaying Available Discounts

**Use Case:** Show customer what discounts are available

```javascript
async function getCustomerDiscounts(appId, customerId, cart) {
  const customer = await billingService.getCustomerById(appId, customerId);

  const availableDiscounts = await billingService.getAvailableDiscounts(
    appId,
    customerId,
    { category: customer.metadata?.category }
  );

  // Filter to relevant discounts
  const relevantDiscounts = availableDiscounts.filter(discount => {
    // Check product categories
    if (discount.productCategories && discount.productCategories.length > 0) {
      const cartTypes = cart.items.map(i => i.type);
      const hasMatch = cartTypes.some(t => discount.productCategories.includes(t));
      if (!hasMatch) return false;
    }

    // Check minimum quantity
    if (discount.minQuantity) {
      const totalQty = cart.items.reduce((sum, i) => sum + i.quantity, 0);
      if (totalQty < discount.minQuantity) return false;
    }

    return true;
  });

  return relevantDiscounts;
}
```

### Pattern 5: One-Time Charge with Discount

**Use Case:** Apply discount to a one-time charge

```javascript
async function createChargeWithDiscount(appId, customerId, amountCents, couponCode) {
  // 1. Calculate discount
  const discountResult = await billingService.calculateDiscounts(
    appId,
    customerId,
    amountCents,
    { couponCodes: couponCode ? [couponCode] : [] }
  );

  // 2. Calculate tax
  const taxResult = await billingService.calculateTax(
    appId,
    customerId,
    discountResult.subtotalAfterDiscount
  );

  // 3. Create charge
  const totalCents = discountResult.subtotalAfterDiscount + taxResult.taxAmountCents;

  const charge = await billingService.createOneTimeCharge(appId, {
    customerId,
    amountCents: totalCents,
    metadata: {
      subtotal_cents: amountCents,
      discount_cents: discountResult.totalDiscountCents,
      tax_cents: taxResult.taxAmountCents
    }
  });

  // 4. Record discount
  for (const discount of discountResult.discounts) {
    await billingService.recordDiscount(appId, {
      chargeId: charge.id,
      couponId: discount.couponId,
      customerId,
      discountType: discount.type,
      discountAmountCents: discount.amountCents,
      description: discount.description
    });
  }

  return charge;
}
```

---

## Common Scenarios

### Scenario 1: Setting Up Coupons

**When:** Creating promotional campaigns

```javascript
// Percentage discount for new customers
await createCoupon({
  app_id: 'myapp',
  code: 'WELCOME15',
  coupon_type: 'percentage',
  value: 15,
  active: true,
  customer_segments: ['new'],
  max_redemptions: 1000
});

// Fixed amount discount
await createCoupon({
  app_id: 'myapp',
  code: 'SAVE10',
  coupon_type: 'fixed',
  value: 1000,  // $10.00
  active: true
});

// Volume discount
await createCoupon({
  app_id: 'myapp',
  code: 'BULK',
  coupon_type: 'volume',
  value: 0,
  active: true,
  volume_tiers: [
    { min: 5, max: 9, discount: 5 },
    { min: 10, max: 19, discount: 10 },
    { min: 20, max: null, discount: 15 }
  ],
  stackable: true
});

// Seasonal promotion
await createCoupon({
  app_id: 'myapp',
  code: 'SUMMER25',
  coupon_type: 'percentage',
  value: 25,
  active: true,
  seasonal_start_date: new Date('2025-06-01'),
  seasonal_end_date: new Date('2025-08-31'),
  max_discount_amount_cents: 5000  // Cap at $50
});
```

### Scenario 2: Handling Rejected Coupons

**When:** User enters invalid coupon

```javascript
async function handleCouponEntry(appId, customerId, subtotal, couponCode) {
  const result = await billingService.calculateDiscounts(
    appId,
    customerId,
    subtotal,
    { couponCodes: [couponCode] }
  );

  if (result.rejectedCoupons.length > 0) {
    const rejected = result.rejectedCoupons[0];
    return {
      success: false,
      message: `Coupon "${rejected.code}" cannot be applied: ${rejected.reason}`
    };
  }

  if (result.discounts.length === 0) {
    return {
      success: false,
      message: 'No discounts applied'
    };
  }

  return {
    success: true,
    discount: result.discounts[0],
    newSubtotal: result.subtotalAfterDiscount
  };
}
```

### Scenario 3: Multiple Stacking Discounts

**When:** Customer has multiple valid coupons

```javascript
const result = await billingService.calculateDiscounts(
  'myapp',
  customerId,
  10000,  // $100.00
  {
    couponCodes: ['VIP20', 'LOYALTY5', 'BULK']
    // VIP20: 20% off, non-stackable, priority 10
    // LOYALTY5: 5% off, stackable, priority 5
    // BULK: volume discount, stackable, priority -1
  }
);

console.log('Applied discounts:', result.appliedInOrder);
// Output: ['VIP20', 'LOYALTY5', 'BULK']

console.log('Rejected:', result.rejectedCoupons);
// Empty if all valid

// Stacking result:
// VIP20: $20 off (non-stackable, wins)
// LOYALTY5: 5% of $80 = $4 off (stackable, on remaining)
// BULK: $0 if quantity doesn't meet tier
// Total: $24 off
```

### Scenario 4: Querying Discount History

**When:** Audit or reporting

```javascript
// Get all discounts for an invoice
const discounts = await billingService.getDiscountsForInvoice('myapp', invoiceId);

// Calculate totals
const totalDiscounted = discounts.reduce((sum, d) => sum + d.discount_amount_cents, 0);

// Group by type
const byType = discounts.reduce((acc, d) => {
  acc[d.discount_type] = (acc[d.discount_type] || 0) + d.discount_amount_cents;
  return acc;
}, {});

console.log('Discount breakdown:', byType);
// Output: { percentage: 2000, volume: 500 }
```

### Scenario 5: Product-Specific Discounts

**When:** Discount only applies to certain products

```javascript
// Create product-specific coupon
await createCoupon({
  app_id: 'myapp',
  code: 'PREMIUM20',
  coupon_type: 'percentage',
  value: 20,
  active: true,
  product_categories: ['premium', 'enterprise']  // Only these products
});

// Apply with product context
const result = await billingService.calculateDiscounts(
  'myapp',
  customerId,
  10000,
  {
    couponCodes: ['PREMIUM20'],
    products: [
      { type: 'standard', quantity: 2 },   // Not eligible
      { type: 'premium', quantity: 1 }     // Eligible
    ]
  }
);

// Coupon applies because cart contains 'premium' product
```

---

## REST API Endpoints

### POST /api/billing/discounts/validate

Validate a coupon code.

**Request:**
```bash
curl -X POST \
  http://localhost:3000/api/billing/discounts/validate \
  -H "x-app-id: myapp" \
  -H "Content-Type: application/json" \
  -d '{
    "coupon_code": "SUMMER20",
    "context": {
      "category": "residential",
      "product_types": ["standard"],
      "total_quantity": 5
    }
  }'
```

**Response (Valid):**
```json
{
  "valid": true,
  "coupon": {
    "id": 1,
    "code": "SUMMER20",
    "type": "percentage",
    "value": 20,
    "description": "20% off"
  }
}
```

**Response (Invalid):**
```json
{
  "valid": false,
  "reason": "Coupon has expired"
}
```

### POST /api/billing/discounts/calculate

Calculate discounts for a cart.

**Request:**
```bash
curl -X POST \
  http://localhost:3000/api/billing/discounts/calculate \
  -H "x-app-id: myapp" \
  -H "Content-Type: application/json" \
  -d '{
    "customer_id": 123,
    "subtotal_cents": 10000,
    "coupon_codes": ["SUMMER20"],
    "products": [
      { "type": "standard", "quantity": 3 }
    ]
  }'
```

**Response:**
```json
{
  "discounts": [
    {
      "coupon_id": 1,
      "code": "SUMMER20",
      "type": "percentage",
      "amount_cents": 2000,
      "description": "20% off"
    }
  ],
  "total_discount_cents": 2000,
  "subtotal_before_discount": 10000,
  "subtotal_after_discount": 8000,
  "rejected_coupons": []
}
```

### GET /api/billing/discounts/available

Get available discounts for customer.

**Request:**
```bash
curl -X GET \
  "http://localhost:3000/api/billing/discounts/available?customer_id=123" \
  -H "x-app-id: myapp"
```

**Response:**
```json
{
  "discounts": [
    {
      "id": 1,
      "code": "SUMMER20",
      "type": "percentage",
      "value": 20,
      "description": "20% off",
      "stackable": false,
      "seasonal_start": "2025-06-01",
      "seasonal_end": "2025-08-31"
    },
    {
      "id": 2,
      "code": "BULK",
      "type": "volume",
      "description": "Volume discount",
      "stackable": true,
      "min_quantity": 5
    }
  ]
}
```

### GET /api/billing/discounts/invoice/:invoiceId

Get discounts applied to an invoice.

**Request:**
```bash
curl -X GET \
  http://localhost:3000/api/billing/discounts/invoice/456 \
  -H "x-app-id: myapp"
```

**Response:**
```json
{
  "discount_applications": [
    {
      "id": 1,
      "invoice_id": 456,
      "discount_type": "percentage",
      "discount_amount_cents": 2000,
      "description": "20% off Summer Sale",
      "coupon": {
        "id": 1,
        "code": "SUMMER20"
      }
    }
  ]
}
```

---

## Troubleshooting

### Issue: Coupon not applying

**Symptom:** `calculateDiscounts()` returns empty discounts array

**Debugging:**
```javascript
const result = await billingService.calculateDiscounts(appId, customerId, subtotal, {
  couponCodes: ['MYCODE']
});

// Check rejected coupons
console.log('Rejected:', result.rejectedCoupons);
// Common reasons:
// - "Coupon not found"
// - "Coupon has expired"
// - "Coupon is not active"
// - "Coupon redemption limit reached"
// - "Minimum 5 items required"
// - "Discount only for commercial customers"

// Validate directly
const validation = await billingService.validateCoupon(appId, 'MYCODE', {
  category: customerCategory,
  productTypes: ['standard'],
  totalQuantity: 3
});
console.log('Validation:', validation);
```

### Issue: Wrong discount amount

**Symptom:** Discount amount doesn't match expected

**Debugging:**
```javascript
const result = await billingService.calculateDiscounts(appId, customerId, subtotal, options);

// Check each discount
result.discounts.forEach(d => {
  console.log(`${d.code}: ${d.type} = ${d.amountCents}c`);
  console.log('  metadata:', d.metadata);
});

// For percentage discounts:
// amount = subtotal * (value / 100)

// For volume discounts:
// Check which tier was applied
const volumeDiscount = result.discounts.find(d => d.type === 'volume');
console.log('Volume tier:', volumeDiscount?.metadata?.tier);
```

### Issue: Stacking not working as expected

**Symptom:** Multiple coupons entered but not all applying

**Common causes:**
1. Non-stackable coupons conflict
2. Priority ordering

**Solution:**
```javascript
// Check coupon stackability
const coupon = await getCoupon('MYCODE');
console.log('Stackable:', coupon.stackable);
console.log('Priority:', coupon.priority);

// Rule: Only one non-stackable discount can apply
// Higher priority wins among non-stackable
// All stackable discounts apply (after non-stackable)
```

### Issue: Seasonal coupon not valid

**Symptom:** "Promotion has ended" or "Promotion starts..."

**Solution:**
```javascript
// Check date range
const coupon = await getCoupon('SEASONAL');
console.log('Start:', coupon.seasonal_start_date);
console.log('End:', coupon.seasonal_end_date);
console.log('Now:', new Date());

// Ensure server timezone matches expected
```

### Issue: Customer segment mismatch

**Symptom:** "Discount only for X customers"

**Solution:**
```javascript
// Check customer's category
const customer = await billingService.getCustomerById(appId, customerId);
console.log('Customer category:', customer.metadata?.category);

// Check coupon's allowed segments
const coupon = await getCoupon('MYCODE');
console.log('Allowed segments:', coupon.customer_segments);

// Pass category explicitly
const result = await billingService.calculateDiscounts(appId, customerId, subtotal, {
  couponCodes: ['MYCODE'],
  category: 'residential'  // Override
});
```

---

## Best Practices

### ✅ DO

1. **Calculate discounts before tax**
   ```javascript
   const discounts = await calculateDiscounts(...);
   const tax = await calculateTax(..., discounts.subtotalAfterDiscount);
   ```

2. **Record discount applications**
   ```javascript
   for (const discount of result.discounts) {
     await recordDiscount(appId, { invoiceId, ...discount });
   }
   ```

3. **Validate coupons before checkout**
   ```javascript
   const validation = await validateCoupon(appId, code, context);
   if (!validation.valid) showError(validation.reason);
   ```

4. **Use appropriate stacking settings**
   - Major promos: `stackable: false, priority: 10+`
   - Loyalty: `stackable: true, priority: 5`
   - Volume: `stackable: true, priority: -1`

5. **Set maximum discount caps**
   ```javascript
   { max_discount_amount_cents: 5000 }  // Cap at $50
   ```

### ❌ DON'T

1. **Don't calculate tax before discounts**
   ```javascript
   // WRONG
   const tax = await calculateTax(..., subtotal);
   const discounts = await calculateDiscounts(...);
   ```

2. **Don't skip recording discounts**
   ```javascript
   // WRONG - no audit trail
   const result = await calculateDiscounts(...);
   // Missing: recordDiscount()
   ```

3. **Don't hardcode discount logic**
   ```javascript
   // WRONG - use database-driven coupons
   const discount = subtotal * 0.20;
   ```

4. **Don't assume single discount**
   ```javascript
   // WRONG - may have multiple
   const discount = result.discounts[0].amountCents;

   // RIGHT
   const discount = result.totalDiscountCents;
   ```

---

## Next Steps

1. **Create coupons** for your promotional campaigns
2. **Integrate discount calculation** into invoice generation
3. **Add coupon validation** to checkout flow
4. **Set up volume discounts** for quantity-based pricing
5. **Monitor discount usage** with audit trail queries

For detailed API reference, see [DISCOUNTSERVICE-API-REFERENCE.md](./DISCOUNTSERVICE-API-REFERENCE.md)

---

**Last Updated:** 2025-02-04
**Phase 2 Status:** Production Ready
