const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError } = require('../utils/errors');
const DiscountCalculator = require('./helpers/DiscountCalculator');
const CouponValidator = require('./helpers/CouponValidator');

/**
 * DiscountService - Phase 2: Discount & Promotion Engine
 *
 * Orchestrates discount calculation workflow:
 * - Fetches coupons, validates eligibility (via CouponValidator)
 * - Calculates amounts (via DiscountCalculator)
 * - Applies stacking rules (via DiscountCalculator)
 * - Records discount applications (CRUD)
 *
 * @author CloudyCastle
 * @phase 2
 */
class DiscountService {
  /**
   * Calculate applicable discounts for a billing scenario
   * @param {string} appId - Application identifier
   * @param {number} customerId - Customer ID
   * @param {number} subtotalCents - Pre-discount amount in cents
   * @param {Object} options - Discount context
   * @param {string[]} options.couponCodes - User-provided coupon codes
   * @param {string} options.category - Customer category (e.g., 'residential', 'commercial')
   * @param {Array<Object>} options.products - Products [{type, quantity}]
   * @param {Object} options.metadata - Additional context for industry-specific logic
   * @returns {Promise<Object>} Discount calculation result
   */
  async calculateDiscounts(appId, customerId, subtotalCents, options = {}) {
    // Validate inputs
    if (!appId || !customerId) {
      throw new ValidationError('appId and customerId are required');
    }

    if (typeof subtotalCents !== 'number' || subtotalCents < 0) {
      throw new ValidationError('subtotalCents must be a non-negative number');
    }

    const { couponCodes = [], category, products = [], metadata = {} } = options;

    // Get customer
    const customer = await billingPrisma.billing_customers.findFirst({
      where: {
        id: customerId,
        app_id: appId
      }
    });

    if (!customer) {
      throw new NotFoundError(`Customer ${customerId} not found for app ${appId}`);
    }

    // Build discount context
    const context = {
      customerId,
      category: category || customer.metadata?.category || 'default',
      products,
      subtotalCents,
      metadata,
      totalQuantity: products.reduce((sum, p) => sum + (p.quantity || 1), 0),
      productTypes: products.map(p => p.type).filter(Boolean)
    };

    // Collect applicable discounts
    const applicableDiscounts = [];
    const rejectedCoupons = [];

    // Process explicit coupon codes
    for (const code of couponCodes) {
      const coupon = await billingPrisma.billing_coupons.findFirst({
        where: {
          app_id: appId,
          code: code.toUpperCase()
        }
      });

      if (!coupon) {
        rejectedCoupons.push({ code, reason: 'Coupon not found' });
        continue;
      }

      const validation = await CouponValidator.validateEligibility(appId, coupon, context);

      if (!validation.eligible) {
        rejectedCoupons.push({ code, reason: validation.reason });
        continue;
      }

      const discountAmount = DiscountCalculator.calculateAmount(coupon, context);
      applicableDiscounts.push({
        couponId: coupon.id,
        code: coupon.code,
        type: coupon.coupon_type,
        amountCents: discountAmount,
        description: DiscountCalculator.getDescription(coupon),
        stackable: coupon.stackable || false,
        priority: coupon.priority || 0,
        metadata: {
          originalValue: coupon.value,
          couponType: coupon.coupon_type
        }
      });
    }

    // Check for automatic volume discounts
    const volumeDiscount = await CouponValidator.findAutomaticVolumeDiscount(appId, context);
    if (volumeDiscount) {
      applicableDiscounts.push(volumeDiscount);
    }

    // Apply stacking and priority rules
    const finalDiscounts = DiscountCalculator.applyStackingRules(applicableDiscounts, subtotalCents);

    // Calculate totals
    const totalDiscountCents = finalDiscounts.reduce((sum, d) => sum + d.amountCents, 0);
    const subtotalAfterDiscount = Math.max(0, subtotalCents - totalDiscountCents);

    return {
      discounts: finalDiscounts,
      totalDiscountCents,
      subtotalBeforeDiscount: subtotalCents,
      subtotalAfterDiscount,
      appliedInOrder: finalDiscounts.map(d => d.code),
      rejectedCoupons
    };
  }

  /**
   * Validate a coupon code without applying it
   * @param {string} appId - Application identifier
   * @param {string} couponCode - Coupon code to validate
   * @param {Object} context - Discount context
   * @returns {Promise<Object>} Validation result
   */
  async validateCoupon(appId, couponCode, context = {}) {
    if (!appId || !couponCode) {
      throw new ValidationError('appId and couponCode are required');
    }

    const coupon = await billingPrisma.billing_coupons.findFirst({
      where: {
        app_id: appId,
        code: couponCode.toUpperCase()
      }
    });

    if (!coupon) {
      return { valid: false, reason: 'Coupon not found' };
    }

    const validation = await CouponValidator.validateEligibility(appId, coupon, context);

    if (!validation.eligible) {
      return { valid: false, reason: validation.reason };
    }

    return {
      valid: true,
      coupon: {
        id: coupon.id,
        code: coupon.code,
        type: coupon.coupon_type,
        value: coupon.value,
        description: DiscountCalculator.getDescription(coupon)
      }
    };
  }

  /**
   * Record a discount application for audit trail
   * @param {string} appId - Application identifier
   * @param {Object} discountDetails - Discount details
   * @returns {Promise<Object>} Created discount application record
   */
  async recordDiscount(appId, discountDetails) {
    if (!appId || !discountDetails.discountAmountCents) {
      throw new ValidationError('appId and discountAmountCents are required');
    }

    const discountApplication = await billingPrisma.billing_discount_applications.create({
      data: {
        app_id: appId,
        invoice_id: discountDetails.invoiceId || null,
        charge_id: discountDetails.chargeId || null,
        coupon_id: discountDetails.couponId || null,
        customer_id: discountDetails.customerId || null,
        discount_type: discountDetails.discountType || 'coupon',
        discount_amount_cents: discountDetails.discountAmountCents,
        description: discountDetails.description || 'Discount applied',
        quantity: discountDetails.quantity || null,
        category: discountDetails.category || null,
        product_types: discountDetails.productTypes || null,
        metadata: discountDetails.metadata || null,
        created_by: discountDetails.createdBy || 'system'
      }
    });

    logger.info('Discount application recorded', {
      app_id: appId,
      discount_application_id: discountApplication.id,
      invoice_id: discountDetails.invoiceId,
      charge_id: discountDetails.chargeId,
      discount_amount_cents: discountDetails.discountAmountCents
    });

    return discountApplication;
  }

  /**
   * Get discount applications for an invoice
   * @param {string} appId - Application identifier
   * @param {number} invoiceId - Invoice ID
   * @returns {Promise<Array>} Array of discount applications
   */
  async getDiscountsForInvoice(appId, invoiceId) {
    if (!appId || !invoiceId) {
      throw new ValidationError('appId and invoiceId are required');
    }

    const discountApplications = await billingPrisma.billing_discount_applications.findMany({
      where: {
        app_id: appId,
        invoice_id: invoiceId
      },
      include: {
        coupon: true
      },
      orderBy: {
        applied_at: 'asc'
      }
    });

    return discountApplications;
  }

  /**
   * Get available discounts/promotions for a customer
   * @param {string} appId - Application identifier
   * @param {number} customerId - Customer ID
   * @param {Object} context - Optional context for eligibility checking
   * @returns {Promise<Array>} Array of available discounts
   */
  async getAvailableDiscounts(appId, customerId, context = {}) {
    if (!appId) {
      throw new ValidationError('appId is required');
    }

    const now = new Date();

    // Get active coupons
    const coupons = await billingPrisma.billing_coupons.findMany({
      where: {
        app_id: appId,
        active: true,
        OR: [
          { redeem_by: null },
          { redeem_by: { gt: now } }
        ]
      },
      orderBy: {
        priority: 'desc'
      }
    });

    // Filter by eligibility if context provided
    const availableDiscounts = [];

    for (const coupon of coupons) {
      // Check seasonal availability
      if (coupon.seasonal_start_date && now < new Date(coupon.seasonal_start_date)) {
        continue;
      }
      if (coupon.seasonal_end_date && now > new Date(coupon.seasonal_end_date)) {
        continue;
      }

      // Check customer segment if provided
      if (coupon.customer_segments && context.category) {
        if (!coupon.customer_segments.includes(context.category)) {
          continue;
        }
      }

      availableDiscounts.push({
        id: coupon.id,
        code: coupon.code,
        type: coupon.coupon_type,
        value: coupon.value,
        description: DiscountCalculator.getDescription(coupon),
        stackable: coupon.stackable,
        seasonalStart: coupon.seasonal_start_date,
        seasonalEnd: coupon.seasonal_end_date,
        minQuantity: coupon.min_quantity,
        productCategories: coupon.product_categories,
        customerSegments: coupon.customer_segments
      });
    }

    return availableDiscounts;
  }

  // Backward-compatible instance method delegates
  validateCouponEligibility(appId, coupon, context) {
    return CouponValidator.validateEligibility(appId, coupon, context);
  }

  _calculateDiscountAmount(coupon, context) {
    return DiscountCalculator.calculateAmount(coupon, context);
  }

  _calculateVolumeDiscount(quantity, volumeTiers, subtotalCents) {
    return DiscountCalculator.calculateVolumeDiscount(quantity, volumeTiers, subtotalCents);
  }

  _applyStackingRules(discounts, subtotalCents) {
    return DiscountCalculator.applyStackingRules(discounts, subtotalCents);
  }

  _findApplicableTier(quantity, volumeTiers) {
    return DiscountCalculator.findApplicableTier(quantity, volumeTiers);
  }

  _getDiscountDescription(coupon) {
    return DiscountCalculator.getDescription(coupon);
  }

  async _findAutomaticVolumeDiscount(appId, context) {
    return CouponValidator.findAutomaticVolumeDiscount(appId, context);
  }
}

module.exports = DiscountService;
