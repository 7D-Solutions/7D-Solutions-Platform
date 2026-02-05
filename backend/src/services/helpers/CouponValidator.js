const { billingPrisma } = require('../../prisma');
const DiscountCalculator = require('./DiscountCalculator');

/**
 * CouponValidator - Coupon eligibility checking logic
 *
 * Static methods for validating coupon eligibility against
 * various rules (expiry, redemption limits, segments, categories, seasonal dates).
 */
class CouponValidator {
  /**
   * Validate if a coupon is eligible for the given context
   * @param {string} appId - Application identifier
   * @param {Object} coupon - Coupon record
   * @param {Object} context - Discount context { customerId, category, productTypes, totalQuantity }
   * @returns {Promise<Object>} Validation result {eligible, reason}
   */
  static async validateEligibility(appId, coupon, context) {
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
   * Find automatic volume discount (not requiring coupon code)
   * @param {string} appId - Application identifier
   * @param {Object} context - Discount context { totalQuantity, subtotalCents }
   * @returns {Promise<Object|null>} Volume discount object or null
   */
  static async findAutomaticVolumeDiscount(appId, context) {
    const volumeCoupons = await billingPrisma.billing_coupons.findMany({
      where: {
        app_id: appId,
        active: true,
        coupon_type: 'volume',
        volume_tiers: { not: null }
      }
    });

    for (const coupon of volumeCoupons) {
      const validation = await CouponValidator.validateEligibility(appId, coupon, context);
      if (validation.eligible) {
        const discountAmount = DiscountCalculator.calculateVolumeDiscount(
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
              tier: DiscountCalculator.findApplicableTier(context.totalQuantity, coupon.volume_tiers)
            }
          };
        }
      }
    }

    return null;
  }
}

module.exports = CouponValidator;
