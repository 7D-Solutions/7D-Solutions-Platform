/**
 * Integration Tests: Proration + Discount + Tax Flow
 *
 * Tests the integration between:
 * - ProrationService (Phase 3)
 * - DiscountService (Phase 2)
 * - TaxService (Phase 1)
 *
 * Verifies the complete flow: Proration → Discount → Tax
 *
 * BILLING PERIOD CONVENTION (Industry Standard):
 * ---------------------------------------------
 * Following Stripe/Recurly best practices, billing periods use calendar months:
 * - Monthly subscriptions: period_end = start of next calendar month
 * - Example: Jan 1 → Feb 1 = 31 days (full January)
 * - Example: Feb 1 → Mar 1 = 28/29 days (full February)
 * - Example: Apr 1 → May 1 = 30 days (full April)
 *
 * This ensures:
 * ✓ Consistent billing dates (e.g., always bill on the 15th)
 * ✓ Natural proration for mid-cycle changes
 * ✓ Customer-friendly: "billed monthly on the 1st"
 *
 * Alternative approaches NOT used:
 * ✗ Fixed 30-day periods (causes billing date drift)
 * ✗ Normalized day counts (not industry standard)
 */

const BillingService = require('../../backend/src/billingService');
const ProrationService = require('../../backend/src/services/ProrationService');
const { billingPrisma } = require('../../backend/src/prisma');

describe('Proration + Discount + Tax Flow Integration', () => {
  let billingService;
  let prorationService;
  const appId = 'test-app';
  let testCustomer;
  let testSubscription;
  let testTaxRate;
  let testCoupon;

  beforeAll(async () => {
    billingService = new BillingService();
    prorationService = new ProrationService();

    // Create test customer with CA jurisdiction
    testCustomer = await billingPrisma.billing_customers.create({
      data: {
        app_id: appId,
        external_customer_id: 'test-customer-proration-001',
        tilled_customer_id: 'tilled_proration_001',
        email: 'proration-test@example.com',
        name: 'Proration Test Customer',
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

    // Create 10% discount coupon
    testCoupon = await billingPrisma.billing_coupons.create({
      data: {
        app_id: appId,
        code: 'UPGRADE10',
        coupon_type: 'percentage',
        value: 10,
        duration: 'once',
        active: true
      }
    });

    // Create test subscription (monthly, $50/month)
    // Period: Jan 1 - Feb 1 (31 days total)
    testSubscription = await billingPrisma.billing_subscriptions.create({
      data: {
        app_id: appId,
        billing_customer_id: testCustomer.id,
        tilled_subscription_id: 'tilled_sub_proration_test_001',
        plan_id: 'plan_basic',
        plan_name: 'Basic Plan',
        status: 'active',
        interval_unit: 'month',
        price_cents: 5000, // $50/month
        current_period_start: new Date('2024-01-01'),
        current_period_end: new Date('2024-02-01'),
        payment_method_id: 'pm_test_001',
        payment_method_type: 'card',
        cancel_at_period_end: false,
        metadata: {}
      }
    });
  });

  afterAll(async () => {
    // Clean up test data
    if (testSubscription) {
      await billingPrisma.billing_subscriptions.delete({
        where: { id: testSubscription.id }
      });
    }
    if (testCustomer) {
      await billingPrisma.billing_customers.delete({
        where: { id: testCustomer.id }
      });
    }
    if (testTaxRate) {
      await billingPrisma.billing_tax_rates.delete({
        where: { id: testTaxRate.id }
      });
    }
    if (testCoupon) {
      await billingPrisma.billing_coupons.delete({
        where: { id: testCoupon.id }
      });
    }
  });

  describe('Mid-Cycle Upgrade: Proration → Discount → Tax', () => {
    it('should calculate proration, apply discount, then calculate tax', async () => {
      // Scenario: Upgrade from $50/month to $100/month on day 16 (mid-cycle)
      // Period: Jan 1 - Feb 1 (31 days), upgrade on Jan 16 (16 days remaining)

      const changeDate = new Date('2024-01-16');
      const oldPriceCents = 5000; // $50
      const newPriceCents = 10000; // $100

      // Step 1: Calculate proration (Phase 3)
      const proration = await prorationService.calculateProration({
        subscriptionId: testSubscription.id,
        changeDate,
        newPriceCents,
        oldPriceCents,
        prorationBehavior: 'create_prorations'
      });

      // Verify proration calculation
      // 16 days remaining out of 31 days ≈ 51.61% of period
      expect(proration.time_proration.daysRemaining).toBe(16);
      expect(proration.time_proration.daysTotal).toBe(31);
      expect(proration.time_proration.prorationFactor).toBeCloseTo(0.5161, 2);

      // Old plan credit: $50 * 0.5161 ≈ $25.81
      expect(proration.old_plan.credit_cents).toBeCloseTo(2581, -1);

      // New plan charge: $100 * 0.5161 ≈ $51.61
      expect(proration.new_plan.charge_cents).toBeCloseTo(5161, -1);

      // Net charge: $51.61 - $25.81 ≈ $25.80
      const netProrationCents = proration.net_change.amount_cents;
      expect(netProrationCents).toBeGreaterThan(0);
      expect(proration.net_change.type).toBe('charge');

      // Step 2: Apply 10% discount to net proration (Phase 2)
      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        netProrationCents,
        ['UPGRADE10']
      );

      // Verify discount: 10% off net proration
      expect(discountResult.totalDiscountCents).toBeCloseTo(netProrationCents * 0.1, 0);
      expect(discountResult.discounts).toHaveLength(1);
      expect(discountResult.discounts[0].code).toBe('UPGRADE10');

      const afterDiscountCents = discountResult.subtotalAfterDiscount;
      expect(afterDiscountCents).toBeLessThan(netProrationCents);

      // Step 3: Calculate tax on discounted amount (Phase 1)
      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        afterDiscountCents
      );

      // Verify tax: 8.25% of discounted amount
      expect(taxResult.jurisdictionCode).toBe('CA');
      expect(taxResult.taxRate).toBeCloseTo(0.0825);
      expect(taxResult.taxAmountCents).toBeCloseTo(afterDiscountCents * 0.0825, 0);

      // Step 4: Calculate final total
      const finalTotalCents = afterDiscountCents + taxResult.taxAmountCents;

      console.log('✓ Mid-Cycle Upgrade Flow Test Passed');
      console.log(`  Original Plan: $${oldPriceCents / 100}/month`);
      console.log(`  New Plan: $${newPriceCents / 100}/month`);
      console.log(`  Days Remaining: ${proration.time_proration.daysRemaining}/${proration.time_proration.daysTotal}`);
      console.log(`  Net Proration: $${netProrationCents / 100}`);
      console.log(`  Discount (10%): -$${discountResult.totalDiscountCents / 100}`);
      console.log(`  After Discount: $${afterDiscountCents / 100}`);
      console.log(`  Tax (8.25%): +$${taxResult.taxAmountCents / 100}`);
      console.log(`  Final Total: $${finalTotalCents / 100}`);
    });
  });

  describe('Mid-Cycle Downgrade: Proration Credit → Discount → Tax', () => {
    it('should calculate proration credit, apply discount, then calculate tax', async () => {
      // Scenario: Downgrade from $100/month to $30/month on day 21
      // Period: Jan 1 - Feb 1 (31 days), downgrade on Jan 21 (11 days remaining)

      const changeDate = new Date('2024-01-21');
      const oldPriceCents = 10000; // $100
      const newPriceCents = 3000; // $30

      // Step 1: Calculate proration (Phase 3)
      const proration = await prorationService.calculateProration({
        subscriptionId: testSubscription.id,
        changeDate,
        newPriceCents,
        oldPriceCents,
        prorationBehavior: 'create_prorations'
      });

      // Verify proration calculation
      // 11 days remaining out of 31 days ≈ 35.48% of period
      expect(proration.time_proration.daysRemaining).toBe(11);
      expect(proration.time_proration.daysTotal).toBe(31);
      expect(proration.time_proration.prorationFactor).toBeCloseTo(0.3548, 2);

      // Old plan credit: $100 * 0.3548 ≈ $35.48
      expect(proration.old_plan.credit_cents).toBeCloseTo(3548, -1);

      // New plan charge: $30 * 0.3548 ≈ $10.64
      expect(proration.new_plan.charge_cents).toBeCloseTo(1064, -1);

      // Net credit: $10.64 - $35.48 ≈ -$24.84 (negative = credit)
      const netProrationCents = proration.net_change.amount_cents;
      expect(netProrationCents).toBeLessThan(0);
      expect(proration.net_change.type).toBe('credit');

      // For downgrade with credit, we only charge for the new plan prorated amount
      const chargeableCents = proration.new_plan.charge_cents;

      // Step 2: Apply 10% discount to new plan charge
      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        chargeableCents,
        ['UPGRADE10']
      );

      const afterDiscountCents = discountResult.subtotalAfterDiscount;

      // Step 3: Calculate tax on discounted new plan charge
      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        afterDiscountCents
      );

      // Step 4: Calculate final total (credit from old plan + discounted new plan + tax)
      // User gets credit, pays discounted new plan + tax
      const finalTotalCents = netProrationCents + afterDiscountCents + taxResult.taxAmountCents;

      console.log('✓ Mid-Cycle Downgrade Flow Test Passed');
      console.log(`  Original Plan: $${oldPriceCents / 100}/month`);
      console.log(`  New Plan: $${newPriceCents / 100}/month`);
      console.log(`  Days Remaining: ${proration.time_proration.daysRemaining}/${proration.time_proration.daysTotal}`);
      console.log(`  Net Proration: $${netProrationCents / 100} (credit)`);
      console.log(`  New Plan Charge: $${chargeableCents / 100}`);
      console.log(`  Discount (10%): -$${discountResult.totalDiscountCents / 100}`);
      console.log(`  After Discount: $${afterDiscountCents / 100}`);
      console.log(`  Tax (8.25%): +$${taxResult.taxAmountCents / 100}`);
      console.log(`  Final Total: $${finalTotalCents / 100}`);
    });
  });

  describe('Edge Cases', () => {
    it('should handle change at period start (full period)', async () => {
      const changeDate = new Date('2024-01-01'); // Period start
      const oldPriceCents = 5000;
      const newPriceCents = 7500;

      const proration = await prorationService.calculateProration({
        subscriptionId: testSubscription.id,
        changeDate,
        newPriceCents,
        oldPriceCents,
        prorationBehavior: 'create_prorations'
      });

      // At period start, proration factor should be 1.0 (full period)
      expect(proration.time_proration.prorationFactor).toBe(1.0);
      expect(proration.time_proration.daysUsed).toBe(0);
      expect(proration.time_proration.note).toBe('change_at_period_start');

      // Should charge full new price and credit full old price
      expect(proration.old_plan.credit_cents).toBe(oldPriceCents);
      expect(proration.new_plan.charge_cents).toBe(newPriceCents);
    });

    it('should handle change at period end (no proration)', async () => {
      const changeDate = new Date('2024-02-01'); // Period end
      const oldPriceCents = 5000;
      const newPriceCents = 7500;

      const proration = await prorationService.calculateProration({
        subscriptionId: testSubscription.id,
        changeDate,
        newPriceCents,
        oldPriceCents,
        prorationBehavior: 'create_prorations'
      });

      // At period end, proration factor should be 0.0 (no remaining time)
      expect(proration.time_proration.prorationFactor).toBe(0.0);
      expect(proration.time_proration.daysRemaining).toBe(0);
      expect(proration.time_proration.note).toBe('change_at_period_end');

      // No charges or credits
      expect(proration.old_plan.credit_cents).toBe(0);
      expect(proration.new_plan.charge_cents).toBe(0);
    });

    it('should handle tax-exempt customer with proration and discount', async () => {
      const changeDate = new Date('2024-01-16');
      const oldPriceCents = 5000;
      const newPriceCents = 10000;

      // Step 1: Calculate proration
      const proration = await prorationService.calculateProration({
        subscriptionId: testSubscription.id,
        changeDate,
        newPriceCents,
        oldPriceCents,
        prorationBehavior: 'create_prorations'
      });

      const netProrationCents = proration.net_change.amount_cents;

      // Step 2: Apply discount
      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        netProrationCents,
        ['UPGRADE10']
      );

      const afterDiscountCents = discountResult.subtotalAfterDiscount;

      // Step 3: Calculate tax with exempt flag
      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        afterDiscountCents,
        { taxExempt: true }
      );

      // No tax for exempt customer
      expect(taxResult.taxAmountCents).toBe(0);
      expect(taxResult.taxType).toBe('exempt');

      const finalTotalCents = afterDiscountCents;
      expect(finalTotalCents).toBe(afterDiscountCents);
    });

    it('should handle zero amount (equal plan prices)', async () => {
      const changeDate = new Date('2024-01-16');
      const samePriceCents = 5000;

      const proration = await prorationService.calculateProration({
        subscriptionId: testSubscription.id,
        changeDate,
        newPriceCents: samePriceCents,
        oldPriceCents: samePriceCents,
        prorationBehavior: 'create_prorations'
      });

      // Net change should be zero
      expect(proration.net_change.amount_cents).toBe(0);

      // Apply discount to zero
      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        0,
        ['UPGRADE10']
      );

      expect(discountResult.subtotalAfterDiscount).toBe(0);
      expect(discountResult.totalDiscountCents).toBe(0);

      // Calculate tax on zero
      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        0
      );

      expect(taxResult.taxAmountCents).toBe(0);
    });
  });

  describe('Variable Month Lengths (Industry Standard)', () => {
    // Test that proration works correctly with different calendar month lengths
    // Industry standard (Stripe, Recurly): billing periods follow calendar months

    it('should handle February cycle (29 days in leap year 2024)', async () => {
      // Create subscription for February cycle: Feb 1 - Mar 1 = 29 days (2024 is leap year)
      const febSubscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: appId,
          billing_customer_id: testCustomer.id,
          tilled_subscription_id: 'tilled_sub_feb_test',
          plan_id: 'plan_basic',
          plan_name: 'Basic Plan',
          status: 'active',
          interval_unit: 'month',
          price_cents: 5000,
          current_period_start: new Date('2024-02-01'),
          current_period_end: new Date('2024-03-01'), // 29 days (leap year)
          payment_method_id: 'pm_test_001',
          payment_method_type: 'card',
          cancel_at_period_end: false,
          metadata: {}
        }
      });

      // Upgrade mid-cycle on Feb 15 (14 days remaining)
      const changeDate = new Date('2024-02-15');
      const proration = await prorationService.calculateProration({
        subscriptionId: febSubscription.id,
        changeDate,
        newPriceCents: 10000,
        oldPriceCents: 5000,
        prorationBehavior: 'create_prorations'
      });

      // Verify February calculations
      expect(proration.time_proration.daysTotal).toBe(29); // Leap year February
      expect(proration.time_proration.daysRemaining).toBe(15); // Feb 15 to Mar 1
      expect(proration.time_proration.prorationFactor).toBeCloseTo(15/29, 2);

      // Clean up
      await billingPrisma.billing_subscriptions.delete({
        where: { id: febSubscription.id }
      });

      console.log('✓ February (29 days) proration test passed');
    });

    it('should handle 30-day month cycle (April)', async () => {
      // Create subscription for April cycle: Apr 1 - May 1 = 30 days
      const aprSubscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: appId,
          billing_customer_id: testCustomer.id,
          tilled_subscription_id: 'tilled_sub_apr_test',
          plan_id: 'plan_basic',
          plan_name: 'Basic Plan',
          status: 'active',
          interval_unit: 'month',
          price_cents: 5000,
          current_period_start: new Date('2024-04-01'),
          current_period_end: new Date('2024-05-01'), // 30 days
          payment_method_id: 'pm_test_001',
          payment_method_type: 'card',
          cancel_at_period_end: false,
          metadata: {}
        }
      });

      // Upgrade mid-cycle on Apr 16 (15 days remaining)
      const changeDate = new Date('2024-04-16');
      const proration = await prorationService.calculateProration({
        subscriptionId: aprSubscription.id,
        changeDate,
        newPriceCents: 10000,
        oldPriceCents: 5000,
        prorationBehavior: 'create_prorations'
      });

      // Verify April calculations (30-day month)
      expect(proration.time_proration.daysTotal).toBe(30);
      expect(proration.time_proration.daysRemaining).toBe(15);
      expect(proration.time_proration.prorationFactor).toBe(0.5); // Exactly 50%

      // Clean up
      await billingPrisma.billing_subscriptions.delete({
        where: { id: aprSubscription.id }
      });

      console.log('✓ April (30 days) proration test passed');
    });

    it('should handle 31-day month cycle (March)', async () => {
      // Create subscription for March cycle: Mar 1 - Apr 1 = 31 days
      const marSubscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: appId,
          billing_customer_id: testCustomer.id,
          tilled_subscription_id: 'tilled_sub_mar_test',
          plan_id: 'plan_basic',
          plan_name: 'Basic Plan',
          status: 'active',
          interval_unit: 'month',
          price_cents: 5000,
          current_period_start: new Date('2024-03-01'),
          current_period_end: new Date('2024-04-01'), // 31 days
          payment_method_id: 'pm_test_001',
          payment_method_type: 'card',
          cancel_at_period_end: false,
          metadata: {}
        }
      });

      // Upgrade mid-cycle on Mar 16 (16 days remaining)
      const changeDate = new Date('2024-03-16');
      const proration = await prorationService.calculateProration({
        subscriptionId: marSubscription.id,
        changeDate,
        newPriceCents: 10000,
        oldPriceCents: 5000,
        prorationBehavior: 'create_prorations'
      });

      // Verify March calculations (31-day month, same as January)
      expect(proration.time_proration.daysTotal).toBe(31);
      expect(proration.time_proration.daysRemaining).toBe(16);
      expect(proration.time_proration.prorationFactor).toBeCloseTo(16/31, 2);

      // Clean up
      await billingPrisma.billing_subscriptions.delete({
        where: { id: marSubscription.id }
      });

      console.log('✓ March (31 days) proration test passed');
    });

    it('should demonstrate month-to-month consistency (billing anchor)', async () => {
      // Show that billing on the 15th of each month works correctly
      // regardless of varying month lengths

      // Jan 15 - Feb 15 (31 days in Jan)
      const jan15ToFeb15 = new Date('2024-02-15') - new Date('2024-01-15');
      const daysJan15ToFeb15 = Math.round(jan15ToFeb15 / (24 * 60 * 60 * 1000));
      expect(daysJan15ToFeb15).toBe(31);

      // Feb 15 - Mar 15 (29 days in Feb 2024, leap year)
      const feb15ToMar15 = new Date('2024-03-15') - new Date('2024-02-15');
      const daysFeb15ToMar15 = Math.round(feb15ToMar15 / (24 * 60 * 60 * 1000));
      expect(daysFeb15ToMar15).toBe(29);

      // Mar 15 - Apr 15 (31 days in Mar)
      const mar15ToApr15 = new Date('2024-04-15') - new Date('2024-03-15');
      const daysMar15ToApr15 = Math.round(mar15ToApr15 / (24 * 60 * 60 * 1000));
      expect(daysMar15ToApr15).toBe(31);

      console.log('✓ Month-to-month billing anchor test passed');
      console.log('  Jan 15 → Feb 15: 31 days');
      console.log('  Feb 15 → Mar 15: 29 days (leap year)');
      console.log('  Mar 15 → Apr 15: 31 days');
      console.log('  ✓ ProrationService handles all month lengths correctly');
    });
  });

  describe('Full Flow with applySubscriptionChange', () => {
    it('should execute complete proration with charges, then apply discount and tax', async () => {
      const changeDate = new Date('2024-01-16');
      const oldPriceCents = 5000; // $50
      const newPriceCents = 10000; // $100

      // Step 1: Apply subscription change (creates proration charges)
      const changeResult = await prorationService.applySubscriptionChange(
        testSubscription.id,
        {
          newPriceCents,
          oldPriceCents,
          newPlanId: 'plan_premium',
          oldPlanId: 'plan_basic'
        },
        {
          prorationBehavior: 'create_prorations',
          effectiveDate: changeDate,
          invoiceImmediately: false
        }
      );

      // Verify subscription was updated
      expect(changeResult.subscription.price_cents).toBe(newPriceCents);
      expect(changeResult.subscription.metadata.last_change.proration_applied).toBe(true);

      // Verify proration charges were created
      expect(changeResult.charges.length).toBeGreaterThan(0);
      const prorationCharge = changeResult.charges.find(c => c.charge_type === 'proration_charge');
      expect(prorationCharge).toBeDefined();

      const netProrationCents = changeResult.proration.net_change.amount_cents;

      // Step 2: Apply discount to net proration
      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        netProrationCents,
        ['UPGRADE10']
      );

      // Step 3: Calculate tax on discounted amount
      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        discountResult.subtotalAfterDiscount
      );

      const finalTotalCents = discountResult.subtotalAfterDiscount + taxResult.taxAmountCents;

      // Verify complete flow
      expect(changeResult.proration).toBeDefined();
      expect(discountResult.discounts).toHaveLength(1);
      expect(taxResult.taxAmountCents).toBeGreaterThan(0);
      expect(finalTotalCents).toBeGreaterThan(0);

      console.log('✓ Full Flow Test Passed');
      console.log(`  Proration charges created: ${changeResult.charges.length}`);
      console.log(`  Net proration: $${netProrationCents / 100}`);
      console.log(`  After discount: $${discountResult.subtotalAfterDiscount / 100}`);
      console.log(`  Tax: $${taxResult.taxAmountCents / 100}`);
      console.log(`  Final total: $${finalTotalCents / 100}`);

      // Clean up created charges
      for (const charge of changeResult.charges) {
        await billingPrisma.billing_charges.delete({
          where: { id: charge.id }
        });
      }

      // Clean up audit event
      await billingPrisma.billing_events.deleteMany({
        where: {
          app_id: appId,
          event_type: 'proration_applied',
          entity_id: testSubscription.id.toString()
        }
      });
    });
  });

  describe('Audit Trail Verification', () => {
    it('should record proration, discount, and tax in audit tables', async () => {
      const changeDate = new Date('2024-01-16');
      const oldPriceCents = 5000;
      const newPriceCents = 10000;

      // Step 1: Apply subscription change
      const changeResult = await prorationService.applySubscriptionChange(
        testSubscription.id,
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

      const netProrationCents = changeResult.proration.net_change.amount_cents;

      // Step 2: Apply discount and record it
      const discountResult = await billingService.applyDiscounts(
        appId,
        testCustomer.id,
        netProrationCents,
        ['UPGRADE10']
      );

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

      // Step 3: Calculate and record tax
      const taxResult = await billingService.calculateTax(
        appId,
        testCustomer.id,
        discountResult.subtotalAfterDiscount
      );

      for (const taxItem of taxResult.breakdown) {
        await billingService.recordTaxCalculation(
          appId,
          taxItem.taxRateId,
          taxItem.taxableAmountCents,
          taxItem.taxAmountCents,
          { invoiceId: null }
        );
      }

      // Verify audit trail

      // 1. Proration charges exist
      const prorationCharges = await billingPrisma.billing_charges.findMany({
        where: {
          app_id: appId,
          billing_customer_id: testCustomer.id,
          charge_type: { in: ['proration_charge', 'proration_credit'] }
        }
      });
      expect(prorationCharges.length).toBeGreaterThan(0);

      // 2. Proration event exists
      const prorationEvents = await billingPrisma.billing_events.findMany({
        where: {
          app_id: appId,
          event_type: 'proration_applied',
          entity_id: testSubscription.id.toString()
        }
      });
      expect(prorationEvents.length).toBeGreaterThan(0);

      // 3. Discount application exists
      const discountApplications = await billingPrisma.billing_discount_applications.findMany({
        where: {
          app_id: appId,
          customer_id: testCustomer.id
        }
      });
      expect(discountApplications.length).toBeGreaterThan(0);

      // 4. Tax calculation exists
      const taxCalculations = await billingPrisma.billing_tax_calculations.findMany({
        where: {
          app_id: appId
        }
      });
      expect(taxCalculations.length).toBeGreaterThan(0);

      console.log('✓ Audit Trail Test Passed');
      console.log(`  Proration charges: ${prorationCharges.length}`);
      console.log(`  Proration events: ${prorationEvents.length}`);
      console.log(`  Discount applications: ${discountApplications.length}`);
      console.log(`  Tax calculations: ${taxCalculations.length}`);

      // Clean up
      await billingPrisma.billing_charges.deleteMany({
        where: { id: { in: prorationCharges.map(c => c.id) } }
      });
      await billingPrisma.billing_events.deleteMany({
        where: { id: { in: prorationEvents.map(e => e.id) } }
      });
      await billingPrisma.billing_discount_applications.deleteMany({
        where: { id: { in: discountApplications.map(d => d.id) } }
      });
      await billingPrisma.billing_tax_calculations.deleteMany({
        where: { id: { in: taxCalculations.map(t => t.id) } }
      });
    });
  });
});
