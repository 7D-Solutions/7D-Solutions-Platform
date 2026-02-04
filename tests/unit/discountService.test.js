const DiscountService = require('../../backend/src/services/DiscountService');
const { billingPrisma } = require('../../backend/src/prisma');
const { NotFoundError, ValidationError } = require('../../backend/src/utils/errors');

// Mock Prisma client
jest.mock('../../backend/src/prisma', () => ({
  billingPrisma: {
    billing_customers: {
      findFirst: jest.fn()
    },
    billing_coupons: {
      findFirst: jest.fn(),
      findMany: jest.fn()
    },
    billing_discount_applications: {
      create: jest.fn(),
      count: jest.fn(),
      findMany: jest.fn()
    }
  }
}));

// Mock logger
jest.mock('@fireproof/infrastructure/utils/logger', () => ({
  info: jest.fn(),
  warn: jest.fn(),
  error: jest.fn()
}));

describe('DiscountService', () => {
  let discountService;
  const appId = 'test-app';
  const customerId = 1;

  beforeEach(() => {
    discountService = new DiscountService();
    jest.clearAllMocks();
  });

  // =========================================================
  // calculateDiscounts Tests
  // =========================================================
  describe('calculateDiscounts', () => {
    const subtotalCents = 10000; // $100.00
    const mockCustomer = {
      id: customerId,
      app_id: appId,
      email: 'test@example.com',
      metadata: { category: 'residential' }
    };

    const mockPercentageCoupon = {
      id: 1,
      app_id: appId,
      code: 'SAVE15',
      coupon_type: 'percentage',
      value: 15,
      active: true,
      redeem_by: null,
      max_redemptions: null,
      product_categories: null,
      customer_segments: null,
      min_quantity: null,
      max_discount_amount_cents: null,
      seasonal_start_date: null,
      seasonal_end_date: null,
      volume_tiers: null,
      stackable: false,
      priority: 0
    };

    const mockFixedCoupon = {
      ...mockPercentageCoupon,
      id: 2,
      code: 'FLAT10',
      coupon_type: 'fixed',
      value: 1000 // $10.00
    };

    it('should calculate percentage discount correctly', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(mockPercentageCoupon);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]); // No volume discounts

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['SAVE15'] }
      );

      expect(result.totalDiscountCents).toBe(1500); // 15% of $100
      expect(result.subtotalAfterDiscount).toBe(8500); // $85
      expect(result.discounts).toHaveLength(1);
      expect(result.discounts[0].code).toBe('SAVE15');
      expect(result.discounts[0].amountCents).toBe(1500);
    });

    it('should calculate fixed discount correctly', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(mockFixedCoupon);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['FLAT10'] }
      );

      expect(result.totalDiscountCents).toBe(1000); // $10 flat
      expect(result.subtotalAfterDiscount).toBe(9000); // $90
      expect(result.discounts[0].amountCents).toBe(1000);
    });

    it('should apply max discount cap', async () => {
      const cappedCoupon = {
        ...mockPercentageCoupon,
        value: 50, // 50%
        max_discount_amount_cents: 2000 // Cap at $20
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(cappedCoupon);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['SAVE15'] }
      );

      // 50% of $100 = $50, but capped at $20
      expect(result.totalDiscountCents).toBe(2000);
      expect(result.subtotalAfterDiscount).toBe(8000);
    });

    it('should not exceed subtotal with discount', async () => {
      const bigCoupon = {
        ...mockFixedCoupon,
        value: 15000 // $150 off a $100 order
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(bigCoupon);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['FLAT10'] }
      );

      expect(result.totalDiscountCents).toBe(10000); // Capped at subtotal
      expect(result.subtotalAfterDiscount).toBe(0);
    });

    it('should reject invalid coupon code', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(null);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['INVALID'] }
      );

      expect(result.totalDiscountCents).toBe(0);
      expect(result.rejectedCoupons).toHaveLength(1);
      expect(result.rejectedCoupons[0].code).toBe('INVALID');
      expect(result.rejectedCoupons[0].reason).toBe('Coupon not found');
    });

    it('should reject inactive coupon', async () => {
      const inactiveCoupon = { ...mockPercentageCoupon, active: false };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(inactiveCoupon);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['SAVE15'] }
      );

      expect(result.totalDiscountCents).toBe(0);
      expect(result.rejectedCoupons[0].reason).toBe('Coupon is not active');
    });

    it('should reject expired coupon', async () => {
      const expiredCoupon = {
        ...mockPercentageCoupon,
        redeem_by: new Date('2020-01-01')
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(expiredCoupon);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['SAVE15'] }
      );

      expect(result.totalDiscountCents).toBe(0);
      expect(result.rejectedCoupons[0].reason).toBe('Coupon has expired');
    });

    it('should reject coupon at max redemptions', async () => {
      const maxedCoupon = { ...mockPercentageCoupon, max_redemptions: 100 };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(maxedCoupon);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(100);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['SAVE15'] }
      );

      expect(result.totalDiscountCents).toBe(0);
      expect(result.rejectedCoupons[0].reason).toBe('Coupon redemption limit reached');
    });

    it('should throw ValidationError for missing required fields', async () => {
      await expect(
        discountService.calculateDiscounts(null, customerId, subtotalCents)
      ).rejects.toThrow(ValidationError);

      await expect(
        discountService.calculateDiscounts(appId, null, subtotalCents)
      ).rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError for invalid amount', async () => {
      await expect(
        discountService.calculateDiscounts(appId, customerId, -100)
      ).rejects.toThrow(ValidationError);
    });

    it('should throw NotFoundError for non-existent customer', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(
        discountService.calculateDiscounts(appId, customerId, subtotalCents)
      ).rejects.toThrow(NotFoundError);
    });
  });

  // =========================================================
  // Customer Segment Validation Tests
  // =========================================================
  describe('customer segment validation', () => {
    const subtotalCents = 10000;
    const mockCustomer = {
      id: customerId,
      app_id: appId,
      metadata: { category: 'residential' }
    };

    it('should accept coupon matching customer segment', async () => {
      const segmentCoupon = {
        id: 1,
        app_id: appId,
        code: 'RESIDENTIAL',
        coupon_type: 'percentage',
        value: 10,
        active: true,
        customer_segments: ['residential', 'small_business'],
        product_categories: null,
        stackable: false,
        priority: 0
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(segmentCoupon);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['RESIDENTIAL'] }
      );

      expect(result.totalDiscountCents).toBe(1000);
      expect(result.discounts).toHaveLength(1);
    });

    it('should reject coupon not matching customer segment', async () => {
      const commercialOnlyCoupon = {
        id: 1,
        app_id: appId,
        code: 'COMMERCIAL',
        coupon_type: 'percentage',
        value: 10,
        active: true,
        customer_segments: ['commercial', 'enterprise'],
        product_categories: null
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(commercialOnlyCoupon);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['COMMERCIAL'] }
      );

      expect(result.totalDiscountCents).toBe(0);
      expect(result.rejectedCoupons[0].reason).toContain('commercial or enterprise');
    });
  });

  // =========================================================
  // Product Category Validation Tests
  // =========================================================
  describe('product category validation', () => {
    const subtotalCents = 10000;
    const mockCustomer = { id: customerId, app_id: appId, metadata: {} };

    it('should accept coupon matching product category', async () => {
      const productCoupon = {
        id: 1,
        app_id: appId,
        code: 'PREMIUM',
        coupon_type: 'percentage',
        value: 20,
        active: true,
        product_categories: ['premium', 'deluxe'],
        customer_segments: null,
        stackable: false,
        priority: 0
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(productCoupon);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, {
          couponCodes: ['PREMIUM'],
          products: [{ type: 'premium', quantity: 1 }]
        }
      );

      expect(result.totalDiscountCents).toBe(2000);
    });

    it('should reject coupon not matching product category', async () => {
      const premiumOnlyCoupon = {
        id: 1,
        app_id: appId,
        code: 'PREMIUM',
        coupon_type: 'percentage',
        value: 20,
        active: true,
        product_categories: ['premium', 'deluxe'],
        customer_segments: null
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(premiumOnlyCoupon);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, {
          couponCodes: ['PREMIUM'],
          products: [{ type: 'basic', quantity: 1 }]
        }
      );

      expect(result.totalDiscountCents).toBe(0);
      expect(result.rejectedCoupons[0].reason).toContain('premium, deluxe');
    });
  });

  // =========================================================
  // Seasonal Date Validation Tests
  // =========================================================
  describe('seasonal date validation', () => {
    const subtotalCents = 10000;
    const mockCustomer = { id: customerId, app_id: appId, metadata: {} };

    it('should reject coupon before seasonal start', async () => {
      const futureCoupon = {
        id: 1,
        app_id: appId,
        code: 'FUTURE',
        coupon_type: 'percentage',
        value: 10,
        active: true,
        seasonal_start_date: new Date(Date.now() + 86400000), // Tomorrow
        seasonal_end_date: null
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(futureCoupon);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['FUTURE'] }
      );

      expect(result.totalDiscountCents).toBe(0);
      expect(result.rejectedCoupons[0].reason).toContain('starts');
    });

    it('should reject coupon after seasonal end', async () => {
      const pastCoupon = {
        id: 1,
        app_id: appId,
        code: 'PAST',
        coupon_type: 'percentage',
        value: 10,
        active: true,
        seasonal_start_date: new Date('2020-01-01'),
        seasonal_end_date: new Date('2020-12-31')
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(pastCoupon);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['PAST'] }
      );

      expect(result.totalDiscountCents).toBe(0);
      expect(result.rejectedCoupons[0].reason).toBe('Promotion has ended');
    });
  });

  // =========================================================
  // Volume Discount Tests
  // =========================================================
  describe('volume discounts', () => {
    const subtotalCents = 10000;
    const mockCustomer = { id: customerId, app_id: appId, metadata: {} };

    it('should calculate volume tier discount correctly', async () => {
      const volumeCoupon = {
        id: 1,
        app_id: appId,
        code: 'VOLUME',
        coupon_type: 'volume',
        value: 0,
        active: true,
        volume_tiers: [
          { min: 3, max: 4, discount: 10 },
          { min: 5, max: 9, discount: 15 },
          { min: 10, discount: 25 }
        ],
        stackable: true,
        priority: -1
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(volumeCoupon);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, {
          couponCodes: ['VOLUME'],
          products: [
            { type: 'product-a', quantity: 3 },
            { type: 'product-b', quantity: 2 }
          ]
        }
      );

      // 5 total quantity = 15% discount tier
      expect(result.totalDiscountCents).toBe(1500);
    });

    it('should return 0 for quantity below minimum tier', async () => {
      const volumeCoupon = {
        id: 1,
        app_id: appId,
        code: 'VOLUME',
        coupon_type: 'volume',
        value: 0,
        active: true,
        volume_tiers: [{ min: 5, discount: 15 }],
        stackable: true,
        priority: -1
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(volumeCoupon);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, {
          couponCodes: ['VOLUME'],
          products: [{ type: 'product-a', quantity: 2 }]
        }
      );

      expect(result.totalDiscountCents).toBe(0);
    });
  });

  // =========================================================
  // Minimum Quantity Validation Tests
  // =========================================================
  describe('minimum quantity validation', () => {
    const subtotalCents = 10000;
    const mockCustomer = { id: customerId, app_id: appId, metadata: {} };

    it('should reject coupon below minimum quantity', async () => {
      const minQtyCoupon = {
        id: 1,
        app_id: appId,
        code: 'BULK',
        coupon_type: 'percentage',
        value: 20,
        active: true,
        min_quantity: 5
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(minQtyCoupon);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, {
          couponCodes: ['BULK'],
          products: [{ type: 'product', quantity: 3 }]
        }
      );

      expect(result.totalDiscountCents).toBe(0);
      expect(result.rejectedCoupons[0].reason).toContain('Minimum 5 items');
    });

    it('should accept coupon at minimum quantity', async () => {
      const minQtyCoupon = {
        id: 1,
        app_id: appId,
        code: 'BULK',
        coupon_type: 'percentage',
        value: 20,
        active: true,
        min_quantity: 5,
        stackable: false,
        priority: 0
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(minQtyCoupon);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, {
          couponCodes: ['BULK'],
          products: [{ type: 'product', quantity: 5 }]
        }
      );

      expect(result.totalDiscountCents).toBe(2000);
    });
  });

  // =========================================================
  // Stacking Rules Tests
  // =========================================================
  describe('stacking rules', () => {
    const subtotalCents = 10000;
    const mockCustomer = { id: customerId, app_id: appId, metadata: {} };

    it('should pick best non-stackable discount when multiple apply', async () => {
      const coupon10 = {
        id: 1, app_id: appId, code: 'SAVE10', coupon_type: 'percentage',
        value: 10, active: true, stackable: false, priority: 0
      };
      const coupon15 = {
        id: 2, app_id: appId, code: 'SAVE15', coupon_type: 'percentage',
        value: 15, active: true, stackable: false, priority: 0
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst
        .mockResolvedValueOnce(coupon10)
        .mockResolvedValueOnce(coupon15);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['SAVE10', 'SAVE15'] }
      );

      // Should pick 15% (higher value)
      expect(result.totalDiscountCents).toBe(1500);
      expect(result.discounts).toHaveLength(1);
      expect(result.discounts[0].code).toBe('SAVE15');
    });

    it('should combine stackable discounts', async () => {
      const couponA = {
        id: 1, app_id: appId, code: 'STACK10', coupon_type: 'percentage',
        value: 10, active: true, stackable: true, priority: 0
      };
      const couponB = {
        id: 2, app_id: appId, code: 'STACK5', coupon_type: 'fixed',
        value: 500, active: true, stackable: true, priority: 0
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst
        .mockResolvedValueOnce(couponA)
        .mockResolvedValueOnce(couponB);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['STACK10', 'STACK5'] }
      );

      // Both should apply
      expect(result.discounts).toHaveLength(2);
      expect(result.totalDiscountCents).toBe(1500); // $10 + $5
    });

    it('should respect priority order', async () => {
      const lowPriority = {
        id: 1, app_id: appId, code: 'LOW', coupon_type: 'percentage',
        value: 30, active: true, stackable: false, priority: 1
      };
      const highPriority = {
        id: 2, app_id: appId, code: 'HIGH', coupon_type: 'percentage',
        value: 10, active: true, stackable: false, priority: 10
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_coupons.findFirst
        .mockResolvedValueOnce(lowPriority)
        .mockResolvedValueOnce(highPriority);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);
      billingPrisma.billing_coupons.findMany.mockResolvedValue([]);

      const result = await discountService.calculateDiscounts(
        appId, customerId, subtotalCents, { couponCodes: ['LOW', 'HIGH'] }
      );

      // Should pick HIGH (higher priority) even though LOW has higher discount
      expect(result.discounts).toHaveLength(1);
      expect(result.discounts[0].code).toBe('HIGH');
      expect(result.totalDiscountCents).toBe(1000);
    });
  });

  // =========================================================
  // recordDiscount Tests
  // =========================================================
  describe('recordDiscount', () => {
    it('should create discount application record', async () => {
      const mockRecord = {
        id: 1,
        app_id: appId,
        invoice_id: 100,
        coupon_id: 1,
        discount_type: 'coupon',
        discount_amount_cents: 1500,
        description: '15% off'
      };

      billingPrisma.billing_discount_applications.create.mockResolvedValue(mockRecord);

      const result = await discountService.recordDiscount(appId, {
        invoiceId: 100,
        couponId: 1,
        customerId: 1,
        discountType: 'coupon',
        discountAmountCents: 1500,
        description: '15% off'
      });

      expect(result.id).toBe(1);
      expect(billingPrisma.billing_discount_applications.create).toHaveBeenCalled();
    });

    it('should throw ValidationError for missing required fields', async () => {
      await expect(
        discountService.recordDiscount(null, { discountAmountCents: 1000 })
      ).rejects.toThrow(ValidationError);

      await expect(
        discountService.recordDiscount(appId, {})
      ).rejects.toThrow(ValidationError);
    });
  });

  // =========================================================
  // validateCoupon Tests
  // =========================================================
  describe('validateCoupon', () => {
    const mockCoupon = {
      id: 1,
      app_id: appId,
      code: 'VALID',
      coupon_type: 'percentage',
      value: 10,
      active: true
    };

    it('should return valid for active coupon', async () => {
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(mockCoupon);
      billingPrisma.billing_discount_applications.count.mockResolvedValue(0);

      const result = await discountService.validateCoupon(appId, 'VALID');

      expect(result.valid).toBe(true);
      expect(result.coupon.code).toBe('VALID');
    });

    it('should return invalid for non-existent coupon', async () => {
      billingPrisma.billing_coupons.findFirst.mockResolvedValue(null);

      const result = await discountService.validateCoupon(appId, 'INVALID');

      expect(result.valid).toBe(false);
      expect(result.reason).toBe('Coupon not found');
    });

    it('should throw ValidationError for missing parameters', async () => {
      await expect(
        discountService.validateCoupon(null, 'CODE')
      ).rejects.toThrow(ValidationError);

      await expect(
        discountService.validateCoupon(appId, null)
      ).rejects.toThrow(ValidationError);
    });
  });

  // =========================================================
  // getDiscountsForInvoice Tests
  // =========================================================
  describe('getDiscountsForInvoice', () => {
    it('should return discount applications for invoice', async () => {
      const mockDiscounts = [
        { id: 1, invoice_id: 100, discount_amount_cents: 1500, coupon: { code: 'SAVE15' } },
        { id: 2, invoice_id: 100, discount_amount_cents: 500, coupon: { code: 'EXTRA5' } }
      ];

      billingPrisma.billing_discount_applications.findMany.mockResolvedValue(mockDiscounts);

      const result = await discountService.getDiscountsForInvoice(appId, 100);

      expect(result).toHaveLength(2);
      expect(result[0].discount_amount_cents).toBe(1500);
    });

    it('should throw ValidationError for missing parameters', async () => {
      await expect(
        discountService.getDiscountsForInvoice(null, 100)
      ).rejects.toThrow(ValidationError);

      await expect(
        discountService.getDiscountsForInvoice(appId, null)
      ).rejects.toThrow(ValidationError);
    });
  });

  // =========================================================
  // getAvailableDiscounts Tests
  // =========================================================
  describe('getAvailableDiscounts', () => {
    it('should return available active coupons', async () => {
      const mockCoupons = [
        { id: 1, code: 'SAVE10', coupon_type: 'percentage', value: 10, active: true, priority: 5 },
        { id: 2, code: 'SAVE20', coupon_type: 'percentage', value: 20, active: true, priority: 10 }
      ];

      billingPrisma.billing_coupons.findMany.mockResolvedValue(mockCoupons);

      const result = await discountService.getAvailableDiscounts(appId, customerId);

      expect(result).toHaveLength(2);
      expect(result[0].code).toBe('SAVE10');
    });

    it('should throw ValidationError for missing appId', async () => {
      await expect(
        discountService.getAvailableDiscounts(null)
      ).rejects.toThrow(ValidationError);
    });
  });
});
