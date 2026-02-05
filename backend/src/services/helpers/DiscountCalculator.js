/**
 * DiscountCalculator - Pure calculation logic for discounts
 *
 * Stateless utility functions for discount amount calculation,
 * volume discount tiers, and stacking rules. All methods are static.
 */
class DiscountCalculator {
  /**
   * Calculate discount amount for a coupon
   * @param {Object} coupon - Coupon record
   * @param {Object} context - { subtotalCents, totalQuantity }
   * @returns {number} Discount amount in cents
   */
  static calculateAmount(coupon, context) {
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
        discountCents = DiscountCalculator.calculateVolumeDiscount(totalQuantity, coupon.volume_tiers, subtotalCents);
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
   * @param {number} quantity - Total quantity
   * @param {Array} volumeTiers - Tier definitions [{min, max?, discount}]
   * @param {number} subtotalCents - Subtotal in cents
   * @returns {number} Discount amount in cents
   */
  static calculateVolumeDiscount(quantity, volumeTiers, subtotalCents) {
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
   * Apply stacking and priority rules to discounts
   * @param {Array} discounts - Array of applicable discount objects
   * @param {number} subtotalCents - Original subtotal in cents
   * @returns {Array} Final discounts after stacking rules applied
   */
  static applyStackingRules(discounts, subtotalCents) {
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
   * Find applicable volume tier for a given quantity
   * @param {number} quantity - Item quantity
   * @param {Array} volumeTiers - Tier definitions
   * @returns {Object|null} Matching tier or null
   */
  static findApplicableTier(quantity, volumeTiers) {
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
   * Get human-readable discount description
   * @param {Object} coupon - Coupon record
   * @returns {string} Description
   */
  static getDescription(coupon) {
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

module.exports = DiscountCalculator;
