# TaxService Integration Guide

**Phase 1: Tax Engine**
**For:** Developers integrating tax calculations into billing flows

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

// Step 1: Set up tax rates (one-time setup)
await billingService.createTaxRate(
  'myapp',
  'CA',
  'sales_tax',
  0.0725,  // 7.25%
  { description: 'California Sales Tax' }
);

// Step 2: Calculate tax during invoice generation
const taxResult = await billingService.calculateTax(
  'myapp',
  customerId,
  10000  // $100.00 after discounts
);

// Step 3: Record tax calculation for audit trail
for (const taxItem of taxResult.breakdown) {
  await billingService.recordTaxCalculation(
    'myapp',
    taxItem.taxRateId,
    taxItem.taxableAmountCents,
    taxItem.taxAmountCents,
    { invoiceId: newInvoice.id }
  );
}

// Step 4: Add tax to invoice total
const totalCents = discountedSubtotal + taxResult.taxAmountCents;
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
              ├─> DiscountService (Phase 2)
              └─> TaxService (Phase 1) ← YOU ARE HERE
                      │
                      ├─> calculateTax()
                      ├─> getTaxRatesByJurisdiction()
                      ├─> createTaxRate()
                      ├─> createTaxExemption()
                      └─> recordTaxCalculation()
```

### Data Flow

```
Customer Order
    ↓
Calculate Subtotal
    ↓
Apply Discounts (Phase 2)
    ↓
Calculate Tax ← TaxService.calculateTax()
    ↓
Record Tax Calculation ← TaxService.recordTaxCalculation()
    ↓
Generate Invoice
```

### Database Schema

```
billing_tax_rates
├── id (PK)
├── app_id
├── jurisdiction_code
├── tax_type
├── rate (Decimal 5,4)
├── effective_date
└── expiration_date

billing_tax_calculations
├── id (PK)
├── app_id
├── invoice_id (FK)
├── charge_id (FK)
├── tax_rate_id (FK)
├── taxable_amount
├── tax_amount
├── jurisdiction_code
├── tax_type
└── rate_applied
```

---

## Integration Patterns

### Pattern 1: Invoice Generation with Tax

**Use Case:** Standard invoice with tax calculation

```javascript
async function generateInvoice(appId, customerId, lineItems) {
  // 1. Calculate subtotal
  const subtotalCents = lineItems.reduce((sum, item) => sum + item.priceCents, 0);

  // 2. Calculate tax
  const taxResult = await billingService.calculateTax(
    appId,
    customerId,
    subtotalCents
  );

  // 3. Create invoice
  const invoice = await billingService.createInvoice(appId, customerId, {
    subtotalCents,
    taxCents: taxResult.taxAmountCents,
    totalCents: subtotalCents + taxResult.taxAmountCents
  });

  // 4. Record tax calculations
  for (const taxItem of taxResult.breakdown) {
    await billingService.recordTaxCalculation(
      appId,
      taxItem.taxRateId,
      taxItem.taxableAmountCents,
      taxItem.taxAmountCents,
      { invoiceId: invoice.id }
    );
  }

  return invoice;
}
```

### Pattern 2: Invoice with Discounts and Tax

**Use Case:** Apply discounts, then calculate tax on discounted amount

```javascript
async function generateInvoiceWithDiscounts(appId, customerId, lineItems, couponCodes) {
  // 1. Calculate subtotal
  const subtotalCents = lineItems.reduce((sum, item) => sum + item.priceCents, 0);

  // 2. Apply discounts (Phase 2 - DiscountService)
  const discountResult = await billingService.applyDiscounts(
    appId,
    customerId,
    subtotalCents,
    couponCodes
  );

  // 3. Calculate tax on DISCOUNTED amount
  const taxResult = await billingService.calculateTax(
    appId,
    customerId,
    discountResult.subtotalAfterDiscount  // Tax on discounted amount!
  );

  // 4. Create invoice
  const invoice = await billingService.createInvoice(appId, customerId, {
    subtotalCents,
    discountCents: discountResult.totalDiscountCents,
    taxCents: taxResult.taxAmountCents,
    totalCents: discountResult.subtotalAfterDiscount + taxResult.taxAmountCents
  });

  // 5. Record discounts (Phase 2)
  for (const discount of discountResult.discounts) {
    await billingService.recordDiscount(appId, invoice.id, discount);
  }

  // 6. Record tax calculations
  for (const taxItem of taxResult.breakdown) {
    await billingService.recordTaxCalculation(
      appId,
      taxItem.taxRateId,
      taxItem.taxableAmountCents,
      taxItem.taxAmountCents,
      { invoiceId: invoice.id }
    );
  }

  return invoice;
}
```

### Pattern 3: One-Time Charge with Tax

**Use Case:** Single charge with tax

```javascript
async function createChargeWithTax(appId, customerId, amountCents, description) {
  // 1. Calculate tax
  const taxResult = await billingService.calculateTax(
    appId,
    customerId,
    amountCents
  );

  // 2. Create charge with tax included
  const charge = await billingService.createCharge(appId, customerId, {
    amountCents: amountCents + taxResult.taxAmountCents,
    description,
    metadata: {
      subtotal_cents: amountCents,
      tax_cents: taxResult.taxAmountCents
    }
  });

  // 3. Record tax calculations
  for (const taxItem of taxResult.breakdown) {
    await billingService.recordTaxCalculation(
      appId,
      taxItem.taxRateId,
      taxItem.taxableAmountCents,
      taxItem.taxAmountCents,
      { chargeId: charge.id }
    );
  }

  return charge;
}
```

### Pattern 4: Tax-Exempt Customer

**Use Case:** Handle customers with tax exemption

```javascript
async function createInvoiceForExemptCustomer(appId, customerId, lineItems) {
  // 1. Check if customer is tax exempt
  const customer = await billingService.getCustomerById(appId, customerId);
  const isTaxExempt = customer.metadata?.tax_exemptions?.length > 0;

  // 2. Calculate subtotal
  const subtotalCents = lineItems.reduce((sum, item) => sum + item.priceCents, 0);

  // 3. Calculate tax (will return 0 if exempt)
  const taxResult = await billingService.calculateTax(
    appId,
    customerId,
    subtotalCents,
    { taxExempt: isTaxExempt }
  );

  // 4. Create invoice
  const invoice = await billingService.createInvoice(appId, customerId, {
    subtotalCents,
    taxCents: taxResult.taxAmountCents,
    totalCents: subtotalCents + taxResult.taxAmountCents,
    metadata: {
      tax_exempt: isTaxExempt
    }
  });

  return invoice;
}
```

---

## Common Scenarios

### Scenario 1: Initial Tax Rate Setup

**When:** First-time setup or adding new jurisdiction

```javascript
// Set up California tax rates
await billingService.createTaxRate('myapp', 'CA', 'sales_tax', 0.0725, {
  description: 'California State Sales Tax',
  effectiveDate: new Date('2024-01-01')
});

// Set up New York City (state + local)
await billingService.createTaxRate('myapp', 'NY-NYC', 'sales_tax', 0.08875, {
  description: 'New York City Sales Tax (state + local)'
});

// Set up Texas with multiple tax types
await billingService.createTaxRate('myapp', 'TX', 'sales_tax', 0.0625, {
  description: 'Texas State Sales Tax'
});

await billingService.createTaxRate('myapp', 'TX', 'environmental_fee', 0.0050, {
  description: 'Texas Environmental Fee'
});
```

### Scenario 2: Customer Tax Exemption

**When:** Non-profit or government customer needs exemption

```javascript
// Create tax exemption for customer
await billingService.createTaxExemption(
  'myapp',
  customerId,
  'sales_tax',
  'CA-NONPROFIT-501C3-999888'
);

// Verify exemption was created
const customer = await billingService.getCustomerById('myapp', customerId);
console.log(customer.metadata.tax_exemptions);
// Output: [{ tax_type: 'sales_tax', certificate_number: '...', status: 'active' }]

// Calculate tax for exempt customer
const taxResult = await billingService.calculateTax(
  'myapp',
  customerId,
  10000,
  { taxExempt: true }
);
console.log(taxResult.taxAmountCents); // 0
```

### Scenario 3: Querying Tax Calculations

**When:** Audit, reporting, or invoice details

```javascript
// Get all tax calculations for an invoice
const taxCalcs = await billingService.getTaxCalculationsForInvoice('myapp', invoiceId);

// Calculate total tax
const totalTax = taxCalcs.reduce((sum, calc) => {
  return sum + parseFloat(calc.tax_amount);
}, 0);

// Group by tax type
const taxByType = taxCalcs.reduce((acc, calc) => {
  if (!acc[calc.tax_type]) acc[calc.tax_type] = 0;
  acc[calc.tax_type] += parseFloat(calc.tax_amount);
  return acc;
}, {});

console.log('Tax breakdown:', taxByType);
// Output: { sales_tax: 7.25, environmental_fee: 1.00 }
```

### Scenario 4: Multi-Jurisdiction Tax

**When:** Customer location determines jurisdiction

```javascript
// Store jurisdiction in customer metadata
await billingService.updateCustomer('myapp', customerId, {
  metadata: {
    jurisdiction_code: 'CA-LOSANGELES',
    state: 'CA',
    city: 'Los Angeles'
  }
});

// Tax calculation will use customer's jurisdiction
const taxResult = await billingService.calculateTax('myapp', customerId, 10000);
console.log(taxResult.jurisdictionCode); // 'CA-LOSANGELES'

// Override jurisdiction for special cases
const txTaxResult = await billingService.calculateTax(
  'myapp',
  customerId,
  10000,
  { jurisdictionCode: 'TX' }
);
console.log(txTaxResult.jurisdictionCode); // 'TX'
```

---

## REST API Endpoints

### GET /api/billing/tax-rates/:jurisdictionCode

Get tax rates for a jurisdiction.

**Request:**
```bash
curl -X GET \
  http://localhost:3000/api/billing/tax-rates/CA \
  -H "x-app-id: myapp"
```

**Response:**
```json
{
  "tax_rates": [
    {
      "id": 1,
      "jurisdictionCode": "CA",
      "taxType": "sales_tax",
      "rate": 0.0725,
      "effectiveDate": "2024-01-01T00:00:00.000Z",
      "expirationDate": null,
      "description": "California State Sales Tax"
    }
  ]
}
```

### POST /api/billing/tax-rates

Create a new tax rate (admin).

**Request:**
```bash
curl -X POST \
  http://localhost:3000/api/billing/tax-rates \
  -H "x-app-id: myapp" \
  -H "Content-Type: application/json" \
  -d '{
    "jurisdiction_code": "NY-NYC",
    "tax_type": "sales_tax",
    "rate": 0.08875,
    "description": "New York City Sales Tax"
  }'
```

**Response:**
```json
{
  "tax_rate": {
    "id": 2,
    "app_id": "myapp",
    "jurisdiction_code": "NY-NYC",
    "tax_type": "sales_tax",
    "rate": "0.0888",
    "effective_date": "2025-02-04T...",
    "description": "New York City Sales Tax"
  }
}
```

### POST /api/billing/tax-exemptions

Create tax exemption for customer.

**Request:**
```bash
curl -X POST \
  http://localhost:3000/api/billing/tax-exemptions \
  -H "x-app-id: myapp" \
  -H "Content-Type: application/json" \
  -d '{
    "customer_id": 123,
    "tax_type": "sales_tax",
    "certificate_number": "NONPROFIT-999888"
  }'
```

**Response:**
```json
{
  "tax_exemption": {
    "tax_type": "sales_tax",
    "certificate_number": "NONPROFIT-999888",
    "status": "active",
    "created_at": "2025-02-04T..."
  }
}
```

### GET /api/billing/tax-calculations/invoice/:invoiceId

Get tax calculations for an invoice.

**Request:**
```bash
curl -X GET \
  http://localhost:3000/api/billing/tax-calculations/invoice/456 \
  -H "x-app-id: myapp"
```

**Response:**
```json
{
  "tax_calculations": [
    {
      "id": 1,
      "invoice_id": 456,
      "taxable_amount": "100.00",
      "tax_amount": "8.25",
      "jurisdiction_code": "CA",
      "tax_type": "sales_tax",
      "rate_applied": "0.0825",
      "tax_rate": {
        "id": 1,
        "jurisdiction_code": "CA",
        "description": "California Sales Tax"
      }
    }
  ]
}
```

---

## Troubleshooting

### Issue: No tax rates found

**Symptom:** `calculateTax()` returns `taxAmountCents: 0` with `taxType: 'none'`

**Cause:** No active tax rates for the jurisdiction

**Solution:**
```javascript
// 1. Check customer's jurisdiction
const customer = await billingService.getCustomerById(appId, customerId);
console.log(customer.metadata?.jurisdiction_code);

// 2. Check if tax rates exist for jurisdiction
const rates = await billingService.getTaxRatesByJurisdiction(
  appId,
  customer.metadata?.jurisdiction_code || 'DEFAULT'
);

// 3. Create tax rate if missing
if (rates.length === 0) {
  await billingService.createTaxRate(appId, jurisdictionCode, 'sales_tax', 0.0725);
}
```

### Issue: Tax calculated incorrectly

**Symptom:** Tax amount doesn't match expected value

**Debugging:**
```javascript
const taxResult = await billingService.calculateTax(appId, customerId, 10000);

// Check breakdown for details
console.log('Tax breakdown:', taxResult.breakdown);
// Verify each rate:
// - taxRateId: Which rate was used
// - rate: The tax rate applied
// - taxAmountCents: Tax calculated

// Check if multiple rates are being applied
if (taxResult.breakdown.length > 1) {
  console.log('Multiple tax rates applied:', taxResult.breakdown.map(b => b.taxType));
}

// Verify customer's jurisdiction
const customer = await billingService.getCustomerById(appId, customerId);
console.log('Customer jurisdiction:', customer.metadata?.jurisdiction_code);
```

### Issue: Customer should be tax-exempt but being charged tax

**Symptom:** Tax calculated for exempt customer

**Solution:**
```javascript
// 1. Verify exemption exists
const customer = await billingService.getCustomerById(appId, customerId);
const hasExemption = customer.metadata?.tax_exemptions?.length > 0;

// 2. Pass taxExempt flag
const taxResult = await billingService.calculateTax(
  appId,
  customerId,
  10000,
  { taxExempt: hasExemption }  // ← Must pass this flag!
);
```

### Issue: Wrong jurisdiction being used

**Symptom:** Tax rate from wrong state/city

**Solution:**
```javascript
// Option 1: Update customer metadata
await billingService.updateCustomer(appId, customerId, {
  metadata: {
    jurisdiction_code: 'CA'  // Correct jurisdiction
  }
});

// Option 2: Override jurisdiction in calculation
const taxResult = await billingService.calculateTax(
  appId,
  customerId,
  10000,
  { jurisdictionCode: 'CA' }  // Override jurisdiction
);
```

---

## Best Practices

### ✅ DO

1. **Calculate tax after discounts**
   ```javascript
   const discounted = applyDiscounts(subtotal);
   const tax = await calculateTax(appId, customerId, discounted);
   ```

2. **Record tax calculations for audit trail**
   ```javascript
   for (const taxItem of taxResult.breakdown) {
     await recordTaxCalculation(..., { invoiceId });
   }
   ```

3. **Store jurisdiction in customer metadata**
   ```javascript
   customer.metadata = { jurisdiction_code: 'CA' };
   ```

4. **Use specific tax types**
   ```javascript
   'sales_tax', 'waste_fee', 'environmental_tax'
   ```

5. **Handle tax-exempt customers**
   ```javascript
   { taxExempt: true }
   ```

### ❌ DON'T

1. **Don't calculate tax before discounts**
   ```javascript
   // WRONG - tax on full amount
   const tax = await calculateTax(appId, customerId, subtotal);
   const discounted = applyDiscounts(subtotal);
   ```

2. **Don't skip recording tax calculations**
   ```javascript
   // WRONG - no audit trail
   const tax = await calculateTax(...);
   // Missing: recordTaxCalculation()
   ```

3. **Don't hardcode tax rates in application code**
   ```javascript
   // WRONG - tax rates should be in database
   const taxRate = 0.0725;
   const tax = subtotal * taxRate;
   ```

4. **Don't assume single tax rate**
   ```javascript
   // WRONG - may have multiple rates
   const tax = subtotal * taxResult.taxRate;

   // RIGHT - use returned tax amount
   const tax = taxResult.taxAmountCents;
   ```

---

## Next Steps

1. **Set up tax rates** for your jurisdictions
2. **Update customer records** with jurisdiction codes
3. **Integrate tax calculation** into invoice generation
4. **Test with multiple scenarios** (exempt customers, multiple rates, etc.)
5. **Monitor tax calculations** using audit trail queries

For detailed API reference, see [TAXSERVICE-API-REFERENCE.md](./TAXSERVICE-API-REFERENCE.md)

---

**Last Updated:** 2025-02-04
**Phase 1 Status:** Production Ready
