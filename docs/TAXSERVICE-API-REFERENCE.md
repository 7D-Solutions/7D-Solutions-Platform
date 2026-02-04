# TaxService API Reference

**Phase 1: Tax Engine**
**Version:** 1.0.0
**Status:** Production Ready

## Overview

The TaxService provides comprehensive tax calculation and management functionality for the billing module. It supports jurisdiction-based tax rates, multiple tax types, customer exemptions, and detailed audit trails.

**Key Features:**
- Jurisdiction-based tax calculation
- Multiple tax rates per jurisdiction (sales tax, fees, etc.)
- Customer tax exemption management
- Comprehensive audit trail
- App-scoped security
- Generic design (works for any billing scenario)

---

## Table of Contents

1. [Class: TaxService](#class-taxservice)
2. [Methods](#methods)
   - [calculateTax()](#calculatetax)
   - [getTaxRatesByJurisdiction()](#gettaxratesbyjurisdiction)
   - [createTaxRate()](#createtaxrate)
   - [createTaxExemption()](#createtaxexemption)
   - [recordTaxCalculation()](#recordtaxcalculation)
   - [getTaxCalculationsForInvoice()](#gettaxcalculationsforinvoice)
   - [getTaxCalculationsForCharge()](#gettaxcalculationsforcharge)
3. [Data Models](#data-models)
4. [Error Handling](#error-handling)
5. [Examples](#examples)

---

## Class: TaxService

```javascript
const TaxService = require('./services/TaxService');
const taxService = new TaxService();
```

### Constructor

```javascript
new TaxService()
```

No parameters required. The service uses the global `billingPrisma` client for database operations.

---

## Methods

### calculateTax()

Calculate tax for a given amount and customer.

**Signature:**
```javascript
async calculateTax(appId, customerId, subtotalCents, options = {})
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `customerId` (number, required) - Customer ID
- `subtotalCents` (number, required) - Amount to calculate tax on (in cents, after discounts)
- `options` (object, optional)
  - `taxExempt` (boolean) - Whether customer is tax exempt (default: false)
  - `jurisdictionCode` (string) - Override jurisdiction code (optional)

**Returns:** Promise<Object>
```javascript
{
  taxAmountCents: 825,              // Total tax in cents
  taxRate: 0.0825,                  // Combined tax rate
  jurisdictionCode: 'CA',           // Jurisdiction used
  taxType: 'sales_tax',             // Tax type(s) applied
  breakdown: [                      // Detailed breakdown
    {
      taxRateId: 1,
      taxType: 'sales_tax',
      rate: 0.0725,
      taxableAmountCents: 10000,
      taxAmountCents: 725,
      description: 'California State Sales Tax'
    },
    {
      taxRateId: 2,
      taxType: 'waste_fee',
      rate: 0.0100,
      taxableAmountCents: 10000,
      taxAmountCents: 100,
      description: 'Environmental Waste Fee'
    }
  ]
}
```

**Throws:**
- `ValidationError` - Missing required parameters or invalid amount
- `NotFoundError` - Customer not found

**Example:**
```javascript
const taxResult = await taxService.calculateTax(
  'myapp',
  123,
  10000,  // $100.00
  { taxExempt: false }
);

console.log(`Tax: $${taxResult.taxAmountCents / 100}`);
// Output: Tax: $8.25
```

---

### getTaxRatesByJurisdiction()

Get all active tax rates for a jurisdiction.

**Signature:**
```javascript
async getTaxRatesByJurisdiction(appId, jurisdictionCode)
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `jurisdictionCode` (string, required) - Jurisdiction code (e.g., "CA", "NY-NYC")

**Returns:** Promise<Array<Object>>
```javascript
[
  {
    id: 1,
    jurisdictionCode: 'CA',
    taxType: 'sales_tax',
    rate: 0.0725,
    effectiveDate: '2024-01-01T00:00:00.000Z',
    expirationDate: null,
    description: 'California State Sales Tax',
    metadata: null
  }
]
```

**Throws:**
- `ValidationError` - Missing required parameters

**Example:**
```javascript
const rates = await taxService.getTaxRatesByJurisdiction('myapp', 'CA');

rates.forEach(rate => {
  console.log(`${rate.taxType}: ${rate.rate * 100}%`);
});
```

---

### createTaxRate()

Create a new tax rate (admin operation).

**Signature:**
```javascript
async createTaxRate(appId, jurisdictionCode, taxType, rate, options = {})
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `jurisdictionCode` (string, required) - Jurisdiction code
- `taxType` (string, required) - Type of tax (e.g., "sales_tax", "waste_fee")
- `rate` (number, required) - Tax rate as decimal (0.0825 = 8.25%)
- `options` (object, optional)
  - `effectiveDate` (Date) - When rate becomes active (default: now)
  - `expirationDate` (Date) - When rate expires (default: null)
  - `description` (string) - Human-readable description
  - `metadata` (object) - Additional metadata

**Returns:** Promise<Object> - Created tax rate record

**Throws:**
- `ValidationError` - Missing required fields or invalid rate (must be 0-1)

**Example:**
```javascript
const newRate = await taxService.createTaxRate(
  'myapp',
  'NY-NYC',
  'sales_tax',
  0.08875,  // 8.875%
  {
    effectiveDate: new Date('2025-01-01'),
    description: 'New York City Sales Tax',
    metadata: { source: 'manual_entry' }
  }
);
```

---

### createTaxExemption()

Create a tax exemption for a customer.

**Signature:**
```javascript
async createTaxExemption(appId, customerId, taxType, certificateNumber)
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `customerId` (number, required) - Customer ID
- `taxType` (string, required) - Type of tax exemption
- `certificateNumber` (string, required) - Tax exemption certificate number

**Returns:** Promise<Object>
```javascript
{
  tax_type: 'sales_tax',
  certificate_number: 'EXEMPT-12345',
  status: 'active',
  created_at: '2025-02-04T...'
}
```

**Throws:**
- `ValidationError` - Missing parameters or duplicate exemption
- `NotFoundError` - Customer not found

**Example:**
```javascript
const exemption = await taxService.createTaxExemption(
  'myapp',
  123,
  'sales_tax',
  'CA-EXEMPT-999888'
);

console.log(`Exemption created: ${exemption.certificate_number}`);
```

---

### recordTaxCalculation()

Record a tax calculation for audit trail (called during invoice generation).

**Signature:**
```javascript
async recordTaxCalculation(appId, taxRateId, taxableAmountCents, taxAmountCents, options = {})
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `taxRateId` (number, required) - Tax rate ID that was used
- `taxableAmountCents` (number, required) - Amount that was taxed (in cents)
- `taxAmountCents` (number, required) - Calculated tax amount (in cents)
- `options` (object, optional)
  - `invoiceId` (number) - Invoice ID (if applicable)
  - `chargeId` (number) - Charge ID (if applicable)

**Returns:** Promise<Object> - Tax calculation record

**Throws:**
- `ValidationError` - Missing parameters or invalid amounts
- `NotFoundError` - Tax rate not found

**Example:**
```javascript
const calculation = await taxService.recordTaxCalculation(
  'myapp',
  1,        // tax rate ID
  10000,    // $100.00 taxable amount
  825,      // $8.25 tax amount
  { invoiceId: 456 }
);
```

---

### getTaxCalculationsForInvoice()

Get all tax calculations for an invoice.

**Signature:**
```javascript
async getTaxCalculationsForInvoice(appId, invoiceId)
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `invoiceId` (number, required) - Invoice ID

**Returns:** Promise<Array<Object>> - Tax calculations with tax rate details

**Throws:**
- `ValidationError` - Missing parameters

**Example:**
```javascript
const calculations = await taxService.getTaxCalculationsForInvoice('myapp', 456);

const totalTax = calculations.reduce((sum, calc) => {
  return sum + parseFloat(calc.tax_amount);
}, 0);

console.log(`Total tax on invoice: $${totalTax.toFixed(2)}`);
```

---

### getTaxCalculationsForCharge()

Get all tax calculations for a charge.

**Signature:**
```javascript
async getTaxCalculationsForCharge(appId, chargeId)
```

**Parameters:**
- `appId` (string, required) - Application identifier
- `chargeId` (number, required) - Charge ID

**Returns:** Promise<Array<Object>> - Tax calculations with tax rate details

**Throws:**
- `ValidationError` - Missing parameters

**Example:**
```javascript
const calculations = await taxService.getTaxCalculationsForCharge('myapp', 789);
```

---

## Data Models

### Tax Rate

```javascript
{
  id: 1,
  app_id: 'myapp',
  jurisdiction_code: 'CA',
  tax_type: 'sales_tax',
  rate: 0.0725,                              // Decimal (8.25% = 0.0825)
  effective_date: '2024-01-01T00:00:00.000Z',
  expiration_date: null,                     // null = no expiration
  description: 'California State Sales Tax',
  metadata: { source: 'auto_import' },
  created_at: '2024-01-01T00:00:00.000Z',
  updated_at: '2024-01-01T00:00:00.000Z'
}
```

### Tax Calculation

```javascript
{
  id: 1,
  app_id: 'myapp',
  invoice_id: 456,
  charge_id: null,
  tax_rate_id: 1,
  taxable_amount: 100.00,      // Decimal (dollars)
  tax_amount: 8.25,            // Decimal (dollars)
  jurisdiction_code: 'CA',
  tax_type: 'sales_tax',
  rate_applied: 0.0825,
  created_at: '2025-02-04T...'
}
```

---

## Error Handling

### Error Types

**ValidationError**
- Missing required parameters
- Invalid data types or formats
- Business rule violations (e.g., rate not in 0-1 range)

**NotFoundError**
- Customer not found
- Tax rate not found

### Error Handling Example

```javascript
try {
  const taxResult = await taxService.calculateTax(appId, customerId, amount);
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

### Complete Invoice Generation with Tax

```javascript
// 1. Calculate subtotal from line items
const subtotalCents = 10000; // $100.00

// 2. Apply discounts (if using DiscountService)
const discountResult = await discountService.applyDiscounts(
  appId,
  customerId,
  subtotalCents
);

// 3. Calculate tax on discounted amount
const taxResult = await taxService.calculateTax(
  appId,
  customerId,
  discountResult.subtotalAfterDiscount
);

// 4. Record tax calculations
for (const taxItem of taxResult.breakdown) {
  await taxService.recordTaxCalculation(
    appId,
    taxItem.taxRateId,
    taxItem.taxableAmountCents,
    taxItem.taxAmountCents,
    { invoiceId: newInvoice.id }
  );
}

// 5. Create final invoice
const invoice = {
  subtotal: subtotalCents,
  discount: discountResult.totalDiscountCents,
  tax: taxResult.taxAmountCents,
  total: discountResult.subtotalAfterDiscount + taxResult.taxAmountCents
};
```

### Setting Up Tax Rates for Multiple Jurisdictions

```javascript
// California
await taxService.createTaxRate('myapp', 'CA', 'sales_tax', 0.0725, {
  description: 'California State Sales Tax'
});

// Los Angeles (additional local tax)
await taxService.createTaxRate('myapp', 'CA-LOSANGELES', 'sales_tax', 0.0950, {
  description: 'Los Angeles County Sales Tax'
});

// New York City
await taxService.createTaxRate('myapp', 'NY-NYC', 'sales_tax', 0.08875, {
  description: 'New York City Sales Tax'
});
```

### Managing Customer Tax Exemptions

```javascript
// Create exemption for non-profit organization
await taxService.createTaxExemption(
  'myapp',
  customerId,
  'sales_tax',
  'NONPROFIT-501C3-12345'
);

// Calculate tax for exempt customer
const taxResult = await taxService.calculateTax(
  appId,
  customerId,
  10000,
  { taxExempt: true }
);

console.log(taxResult.taxAmountCents); // 0
```

---

## Integration with BillingService

TaxService is integrated into the BillingService facade:

```javascript
const BillingService = require('./billingService');
const billingService = new BillingService();

// All TaxService methods are available via delegation:
await billingService.calculateTax(appId, customerId, amount);
await billingService.getTaxRatesByJurisdiction(appId, jurisdictionCode);
await billingService.createTaxRate(appId, jurisdictionCode, taxType, rate);
await billingService.createTaxExemption(appId, customerId, taxType, certNumber);
await billingService.recordTaxCalculation(appId, taxRateId, taxable, tax);
await billingService.getTaxCalculationsForInvoice(appId, invoiceId);
await billingService.getTaxCalculationsForCharge(appId, chargeId);
```

---

## Best Practices

1. **Always calculate tax AFTER discounts**
   - Apply discounts first, then calculate tax on the discounted amount
   - This is the standard "discount-before-tax" flow

2. **Store jurisdiction in customer metadata**
   ```javascript
   customer.metadata = {
     jurisdiction_code: 'CA',
     // or
     state: 'CA'
   };
   ```

3. **Record tax calculations for audit trail**
   - Always call `recordTaxCalculation()` after calculating tax
   - Links tax calculations to invoices/charges for compliance

4. **Use specific jurisdiction codes**
   - State: "CA", "NY", "TX"
   - City: "NY-NYC", "CA-LOSANGELES"
   - Format: "{STATE}-{CITY}" or "{STATE}"

5. **Handle tax exemptions properly**
   - Store certificate numbers
   - Pass `taxExempt: true` when calculating tax
   - Verify exemption status before applying

6. **Test with multiple tax rates**
   - Many jurisdictions have state + local taxes
   - Test with combined rates to ensure correct calculation

---

## Testing

See `packages/billing/tests/unit/taxService.test.js` for comprehensive test examples.

**Test Coverage:**
- 45 test cases
- All methods tested
- Edge cases and error scenarios
- Validation testing
- Integration scenarios

**Run Tests:**
```bash
npm test -- taxService.test.js
```

---

## Support

For questions or issues:
- See integration examples in this document
- Check test cases for usage patterns
- Review error handling section for common issues

---

**Last Updated:** 2025-02-04
**Phase 1 Status:** Production Ready
