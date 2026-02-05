/**
 * Integration Tests: Discount + Tax Flow
 *
 * Tests the integration between DiscountService (Phase 2) and TaxService (Phase 1)
 * Verifies the discount-before-tax calculation flow
 */

const BillingService = require('../../backend/src/billingService');
const { billingPrisma } = require('../../backend/src/prisma');
const { setupIntegrationTests, teardownIntegrationTests } = require('./database-cleanup');

describe('Discount + Tax Flow Integration', () => {
  let billingService;
  const appId = 'test-app';
  let testCustomer;
  let testTaxRate;
  let testCoupon;

  beforeAll(async () => {
    await setupIntegrationTests();
    billingService = new BillingService();

    // Create test customer with CA jurisdiction
    testCustomer = await billingPrisma.billing_customers.create({
      data: {
        app_id: appId,
        external_customer_id: 'test-customer-001',
        tilled_customer_id: 'tilled_test_001',
        email: 'test@example.com',
        name: 'Test Customer',
        metadata: {
          jurisdiction_code: 'CA',
          state: 'CA'
        }
      }
    });

    // Create California tax rate (8.25%)
    testTaxRate = await billingPrisma.billing_tax_rates.create({
      data: {
        app_id: appId,
        jurisdiction_code: 'CA',
        tax_type: 'sales_tax',
        rate: 0.0825,
        effective_date: new Date('2024-01-01'),
        expiration_date: null,
        description: 'California Sales Tax'
      }
    });

    // Create 15% discount coupon
    testCoupon = await billingPrisma.billing_coupons.create({
      data: {
        app_id: appId,
        code: 'SAVE15',
        coupon_type: 'percentage',
        value: 15,
        duration: 'once',
        active: true
      }
    });
  });

  // No afterAll cleanup needed — integrationSetup.js beforeAll(cleanDatabase)
  // handles TRUNCATE between test files.

  describe('Standard Flow: Discount Before Tax', () => {
    it('should apply 15% discount then calculate 8.25% tax on discounted amount', async () => {
      const subtotalCents = 10000; // $100.00

      // Step 1: Apply discount (Phase 2 - DiscountService)
      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        subtotalCents,
        ['SAVE15']
      );

      // Verify discount calculation
      expect(discountResult.subtotalAfterDiscount).toBe(8500); // $85.00
      expect(discountResult.totalDiscountCents).toBe(1500); // $15.00
      expect(discountResult.discounts).toHaveLength(1);
      expect(discountResult.discounts[0].code).toBe('SAVE15');

      // Step 2: Calculate tax (Phase 1 - TaxService)
      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        discountResult.subtotalAfterDiscount
      );

      // Verify tax calculation (8.25% of $85)
      expect(taxResult.taxAmountCents).toBe(701); // $7.01
      expect(taxResult.jurisdictionCode).toBe('CA');
      expect(taxResult.taxRate).toBeCloseTo(0.0825);

      // Step 3: Calculate final total
      const totalCents = discountResult.subtotalAfterDiscount + taxResult.taxAmountCents;
      expect(totalCents).toBe(9201); // $92.01

      // Verify calculation breakdown:
      // $100.00 - $15.00 (discount) = $85.00
      // $85.00 + $7.01 (tax) = $92.01
      console.log('✓ Discount + Tax Flow Test Passed');
      console.log(`  Original: $${subtotalCents / 100}`);
      console.log(`  Discount: -$${discountResult.totalDiscountCents / 100}`);
      console.log(`  Subtotal: $${discountResult.subtotalAfterDiscount / 100}`);
      console.log(`  Tax: +$${taxResult.taxAmountCents / 100}`);
      console.log(`  Total: $${totalCents / 100}`);
    });

    it('should work with no discount (invalid coupon code)', async () => {
      const subtotalCents = 10000; // $100.00

      // Apply invalid coupon
      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        subtotalCents,
        ['INVALID_CODE']
      );

      // No discount applied
      expect(discountResult.subtotalAfterDiscount).toBe(10000);
      expect(discountResult.totalDiscountCents).toBe(0);

      // Calculate tax on full amount
      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        discountResult.subtotalAfterDiscount
      );

      // Tax on $100 = $8.25
      expect(taxResult.taxAmountCents).toBe(825);

      const totalCents = discountResult.subtotalAfterDiscount + taxResult.taxAmountCents;
      expect(totalCents).toBe(10825); // $108.25
    });

    it('should handle tax-exempt customer with discount', async () => {
      const subtotalCents = 10000; // $100.00

      // Apply discount
      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        subtotalCents,
        ['SAVE15']
      );

      expect(discountResult.subtotalAfterDiscount).toBe(8500); // $85.00

      // Calculate tax with exempt flag
      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        discountResult.subtotalAfterDiscount,
        { taxExempt: true }
      );

      // No tax for exempt customer
      expect(taxResult.taxAmountCents).toBe(0);
      expect(taxResult.taxType).toBe('exempt');

      const totalCents = discountResult.subtotalAfterDiscount + taxResult.taxAmountCents;
      expect(totalCents).toBe(8500); // $85.00 (discount only, no tax)
    });
  });

  describe('Edge Cases', () => {
    it('should handle zero amount', async () => {
      const subtotalCents = 0;

      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        subtotalCents,
        ['SAVE15']
      );

      expect(discountResult.subtotalAfterDiscount).toBe(0);
      expect(discountResult.totalDiscountCents).toBe(0);

      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        discountResult.subtotalAfterDiscount
      );

      expect(taxResult.taxAmountCents).toBe(0);
    });

    it('should handle very small amounts (rounding)', async () => {
      const subtotalCents = 100; // $1.00

      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        subtotalCents,
        ['SAVE15']
      );

      expect(discountResult.subtotalAfterDiscount).toBe(85); // $0.85

      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        discountResult.subtotalAfterDiscount
      );

      // Tax on $0.85 = $0.07 (rounded)
      expect(taxResult.taxAmountCents).toBeGreaterThanOrEqual(6);
      expect(taxResult.taxAmountCents).toBeLessThanOrEqual(8);
    });

    it('should handle large amounts', async () => {
      const subtotalCents = 100000000; // $1,000,000.00

      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        subtotalCents,
        ['SAVE15']
      );

      expect(discountResult.subtotalAfterDiscount).toBe(85000000); // $850,000

      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        discountResult.subtotalAfterDiscount
      );

      // Tax on $850,000 = $70,125
      expect(taxResult.taxAmountCents).toBe(7012500);
    });
  });

  describe('Multiple Discounts + Tax', () => {
    let secondCoupon;

    beforeAll(async () => {
      // Create a second coupon (10% off)
      secondCoupon = await billingPrisma.billing_coupons.create({
        data: {
          app_id: appId,
          code: 'SAVE10',
          coupon_type: 'percentage',
          value: 10,
          duration: 'once',
          active: true
        }
      });
    });

    // No nested afterAll cleanup needed — TRUNCATE handles it between files.

    it('should apply multiple coupons then calculate tax', async () => {
      const subtotalCents = 10000; // $100.00

      // Apply both coupons
      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        subtotalCents,
        ['SAVE15', 'SAVE10']
      );

      // Assuming coupons stack:
      // 15% + 10% = 25% total discount (or sequential application)
      expect(discountResult.subtotalAfterDiscount).toBeLessThan(10000);
      expect(discountResult.discounts.length).toBeGreaterThan(0);

      // Calculate tax on discounted amount
      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        discountResult.subtotalAfterDiscount
      );

      expect(taxResult.taxAmountCents).toBeGreaterThan(0);

      console.log('Multiple discounts result:', {
        original: subtotalCents,
        afterDiscount: discountResult.subtotalAfterDiscount,
        tax: taxResult.taxAmountCents,
        total: discountResult.subtotalAfterDiscount + taxResult.taxAmountCents
      });
    });
  });

  describe('Audit Trail', () => {
    it('should record both discount and tax for invoice', async () => {
      const subtotalCents = 10000; // $100.00

      // Apply discount
      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        subtotalCents,
        ['SAVE15']
      );

      // Calculate tax
      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        discountResult.subtotalAfterDiscount
      );

      // Record discount (Phase 2) - without invoice reference for this test
      for (const discount of discountResult.discounts) {
        await billingService.recordDiscount(appId, {
          invoiceId: null,
          couponId: testCoupon.id,
          customerId: testCustomer.id,
          discountType: 'coupon',
          discountAmountCents: discount.amountCents,
          description: `Coupon: ${discount.code}`
        });
      }

      // Record tax (Phase 1) - without invoice reference for this test
      for (const taxItem of taxResult.breakdown) {
        await billingService.recordTaxCalculation(
          appId,
          taxItem.taxRateId,
          taxItem.taxableAmountCents,
          taxItem.taxAmountCents,
          { invoiceId: null }
        );
      }

      // Verify both are recorded (check by customer instead of invoice for this test)
      const discounts = await billingPrisma.billing_discount_applications.findMany({
        where: { app_id: appId, customer_id: testCustomer.id }
      });
      const taxes = await billingPrisma.billing_tax_calculations.findMany({
        where: { app_id: appId }
      });

      expect(discounts.length).toBeGreaterThan(0);
      expect(taxes.length).toBeGreaterThan(0);

      // Clean up
      await billingPrisma.billing_discount_applications.deleteMany({
        where: { app_id: appId, customer_id: testCustomer.id }
      });
      await billingPrisma.billing_tax_calculations.deleteMany({
        where: { app_id: appId }
      });

      console.log('✓ Audit trail test passed - both discount and tax recorded');
    });
  });
});
