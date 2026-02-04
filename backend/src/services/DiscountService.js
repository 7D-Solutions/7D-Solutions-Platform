const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError } = require('../utils/errors');

/**
 * DiscountService - Phase 2: Discount & Promotion Engine
 *
 * Generic discount service supporting:
 * - Product-specific discounts (via product_categories)
 * - Category-based discounts (via customer_segments)
 * - Volume discounts (tiered pricing)
 * - Seasonal promotions (date-based)
 * - Referral programs
 * - Contract term discounts
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

      const validation = await this.validateCouponEligibility(appId, coupon, context);

      if (!validation.eligible) {
        rejectedCoupons.push({ code, reason: validation.reason });
        continue;
      }

      const discountAmount = this._calculateDiscountAmount(coupon, context);
      applicableDiscounts.push({
        couponId: coupon.id,
        code: coupon.code,
        type: coupon.coupon_type,
        amountCents: discountAmount,
        description: this._getDiscountDescription(coupon),
        stackable: coupon.stackable || false,
        priority: coupon.priority || 0,
        metadata: {
          originalValue: coupon.value,
          couponType: coupon.coupon_type
        }
      });
    }

    // Check for automatic volume discounts
    const volumeDiscount = await this._findAutomaticVolumeDiscount(appId, context);
    if (volumeDiscount) {
      applicableDiscounts.push(volumeDiscount);
    }

    // Apply stacking and priority rules
    const finalDiscounts = this._applyStackingRules(applicableDiscounts, subtotalCents);

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
   * Validate if a coupon is eligible for the given context
   * @param {string} appId - Application identifier
   * @param {Object} coupon - Coupon record
   * @param {Object} context - Discount context
   * @returns {Promise<Object>} Validation result {eligible, reason}
   */
  async validateCouponEligibility(appId, coupon, context) {
    // Check active status
    if (!coupon.active) {
      return { eligible: false, reason: 'Coupon is not active' };
    }

    // Check expiration date
    if (coupon.redeem_by && new Date() > new Date(coupon.redeem_by)) {
      return { eligible: false, reason: 'Coupon has expired' };
    }

    // Check max redemptions
    if (coupon.max_redemptions) {
      const redemptionCount = await billingPrisma.billing_discount_applications.count({
        where: {
          app_id: appId,
          coupon_id: coupon.id
        }
      });

      if (redemptionCount >= coupon.max_redemptions) {
        return { eligible: false, reason: 'Coupon redemption limit reached' };
      }
    }

    // Check seasonal dates
    const now = new Date();
    if (coupon.seasonal_start_date && now < new Date(coupon.seasonal_start_date)) {
      return {
        eligible: false,
        reason: `Promotion starts ${new Date(coupon.seasonal_start_date).toLocaleDateString()}`
      };
    }

    if (coupon.seasonal_end_date && now > new Date(coupon.seasonal_end_date)) {
      return { eligible: false, reason: 'Promotion has ended' };
    }

    // Check product categories eligibility
    if (coupon.product_categories && Array.isArray(coupon.product_categories)) {
      const hasEligibleProduct = context.productTypes.some(
        type => coupon.product_categories.includes(type)
      );

      if (!hasEligibleProduct && context.productTypes.length > 0) {
        return {
          eligible: false,
          reason: `Discount only applies to: ${coupon.product_categories.join(', ')}`
        };
      }
    }

    // Check customer segment eligibility
    if (coupon.customer_segments && Array.isArray(coupon.customer_segments)) {
      if (!coupon.customer_segments.includes(context.category)) {
        return {
          eligible: false,
          reason: `Discount only for ${coupon.customer_segments.join(' or ')} customers`
        };
      }
    }

    // Check minimum quantity
    if (coupon.min_quantity && context.totalQuantity < coupon.min_quantity) {
      return {
        eligible: false,
        reason: `Minimum ${coupon.min_quantity} items required`
      };
    }

    return { eligible: true };
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

    const validation = await this.validateCouponEligibility(appId, coupon, context);

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
        description: this._getDiscountDescription(coupon)
      }
    };
  }

  /**
   * Record a discount application for audit trail
   * @param {string} appId - Application identifier
   * @param {Object} discountDetails - Discount details
   * @param {number} discountDetails.invoiceId - Invoice ID (optional)
   * @param {number} discountDetails.chargeId - Charge ID (optional)
   * @param {number} discountDetails.couponId - Coupon ID (optional)
   * @param {number} discountDetails.customerId - Customer ID
   * @param {string} discountDetails.discountType - Type of discount
   * @param {number} discountDetails.discountAmountCents - Discount amount
   * @param {string} discountDetails.description - Description
   * @param {Object} discountDetails.metadata - Additional metadata
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
        description: this._getDiscountDescription(coupon),
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

  /**
   * Calculate discount amount for a coupon
   * @private
   */
  _calculateDiscountAmount(coupon, context) {
    const { subtotalCents, totalQuantity } = context;
    let discountCents = 0;

    switch (coupon.coupon_type) {
      case 'percentage':
        discountCents = Math.round(subtotalCents * (coupon.value / 100));
        break;

      case 'fixed':
        discountCents = coupon.value;
        break;

      case 'volume':
        discountCents = this._calculateVolumeDiscount(totalQuantity, coupon.volume_tiers, subtotalCents);
        break;

      default:
        discountCents = coupon.value;
    }

    // Apply maximum discount cap
    if (coupon.max_discount_amount_cents && discountCents > coupon.max_discount_amount_cents) {
      discountCents = coupon.max_discount_amount_cents;
    }

    // Don't exceed subtotal
    return Math.min(discountCents, subtotalCents);
  }

  /**
   * Calculate volume-based discount
   * @private
   */
  _calculateVolumeDiscount(quantity, volumeTiers, subtotalCents) {
    if (!volumeTiers || !Array.isArray(volumeTiers) || volumeTiers.length === 0) {
      return 0;
    }

    // Sort tiers by minimum (ascending)
    const sortedTiers = [...volumeTiers].sort((a, b) => a.min - b.min);

    // Find applicable tier (highest tier customer qualifies for)
    let applicableTier = null;
    for (const tier of sortedTiers) {
      if (quantity >= tier.min) {
        if (!tier.max || quantity <= tier.max) {
          applicableTier = tier;
        }
      }
    }

    if (!applicableTier) {
      return 0;
    }

    // Calculate discount (percentage-based)
    return Math.round(subtotalCents * (applicableTier.discount / 100));
  }

  /**
   * Find automatic volume discount (not requiring coupon code)
   * @private
   */
  async _findAutomaticVolumeDiscount(appId, context) {
    const volumeCoupons = await billingPrisma.billing_coupons.findMany({
      where: {
        app_id: appId,
        active: true,
        coupon_type: 'volume',
        volume_tiers: { not: null }
      }
    });

    for (const coupon of volumeCoupons) {
      const validation = await this.validateCouponEligibility(appId, coupon, context);
      if (validation.eligible) {
        const discountAmount = this._calculateVolumeDiscount(
          context.totalQuantity,
          coupon.volume_tiers,
          context.subtotalCents
        );

        if (discountAmount > 0) {
          return {
            couponId: coupon.id,
            code: coupon.code || 'VOLUME_AUTO',
            type: 'volume',
            amountCents: discountAmount,
            description: `Volume discount: ${context.totalQuantity}+ items`,
            stackable: coupon.stackable || true, // Volume discounts typically stack
            priority: coupon.priority || -1, // Apply after explicit discounts
            metadata: {
              quantity: context.totalQuantity,
              tier: this._findApplicableTier(context.totalQuantity, coupon.volume_tiers)
            }
          };
        }
      }
    }

    return null;
  }

  /**
   * Find applicable volume tier
   * @private
   */
  _findApplicableTier(quantity, volumeTiers) {
    if (!volumeTiers) return null;

    const sortedTiers = [...volumeTiers].sort((a, b) => a.min - b.min);
    for (const tier of sortedTiers) {
      if (quantity >= tier.min && (!tier.max || quantity <= tier.max)) {
        return tier;
      }
    }
    return null;
  }

  /**
   * Apply stacking and priority rules to discounts
   * @private
   */
  _applyStackingRules(discounts, subtotalCents) {
    if (discounts.length === 0) return [];

    // Sort by priority (higher first)
    const sorted = [...discounts].sort((a, b) => b.priority - a.priority);

    // Separate stackable and non-stackable
    const stackable = sorted.filter(d => d.stackable);
    const nonStackable = sorted.filter(d => !d.stackable);

    // For non-stackable, pick the best one
    let bestNonStackable = null;
    if (nonStackable.length > 0) {
      // Among same priority, pick highest discount
      const highestPriority = nonStackable[0].priority;
      const samePriority = nonStackable.filter(d => d.priority === highestPriority);
      bestNonStackable = samePriority.reduce(
        (best, current) => current.amountCents > best.amountCents ? current : best,
        samePriority[0]
      );
    }

    // Combine: best non-stackable + all stackable
    const finalDiscounts = [];

    if (bestNonStackable) {
      finalDiscounts.push(bestNonStackable);
    }

    // Add stackable discounts, recalculating amounts on reduced subtotal
    let remainingSubtotal = subtotalCents - (bestNonStackable?.amountCents || 0);

    for (const discount of stackable) {
      // For stackable discounts, recalculate on remaining amount if percentage
      if (discount.metadata?.couponType === 'percentage') {
        const recalculatedAmount = Math.round(
          remainingSubtotal * (discount.metadata.originalValue / 100)
        );
        discount.amountCents = Math.min(recalculatedAmount, remainingSubtotal);
      }

      finalDiscounts.push(discount);
      remainingSubtotal -= discount.amountCents;
    }

    return finalDiscounts;
  }

  /**
   * Get human-readable discount description
   * @private
   */
  _getDiscountDescription(coupon) {
    switch (coupon.coupon_type) {
      case 'percentage':
        return `${coupon.value}% off`;
      case 'fixed':
        return `$${(coupon.value / 100).toFixed(2)} off`;
      case 'volume':
        return 'Volume discount';
      case 'referral':
        return 'Referral discount';
      case 'contract':
        return `Contract term discount (${coupon.contract_term_months} months)`;
      default:
        return 'Discount';
    }
  }
}

module.exports = DiscountService;
