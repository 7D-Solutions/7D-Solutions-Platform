const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError } = require('../utils/errors');
const ProrationCalculator = require('./helpers/ProrationCalculator');
const ProrationExecutor = require('./helpers/ProrationExecutor');

/**
 * ProrationService - Phase 3: Proration Engine
 *
 * Generic proration engine for handling mid-cycle billing changes:
 * - Plan upgrades/downgrades
 * - Subscription cancellations
 * - Quantity changes
 * - Time-based proration calculations
 *
 * Uses existing billing_charges table with charge_type 'proration_credit' or 'proration_charge'
 * Stores proration details in metadata JSON field
 *
 * Calculation logic delegated to ProrationCalculator (pure math, static methods).
 * Execution logic delegated to ProrationExecutor (DB writes, static methods).
 *
 * @author PearlLynx (plan), LavenderDog (implementation)
 * @phase 3
 */
class ProrationService {
  constructor(getTilledClientFn) {
    this.getTilledClient = getTilledClientFn;
  }

  /**
   * Calculate proration for subscription change
   * @param {Object} params
   * @param {number} params.subscriptionId - Billing subscription ID
   * @param {Date} params.changeDate - When the change takes effect
   * @param {number} params.newPriceCents - New plan price (cents)
   * @param {number} params.oldPriceCents - Current plan price (cents)
   * @param {number} params.newQuantity - New quantity (optional)
   * @param {number} params.oldQuantity - Current quantity (optional)
   * @param {string} params.prorationBehavior - 'create_prorations' | 'none' | 'always_invoice'
   * @param {string} params.appId - App ID for multi-tenant scoping
   * @returns {Promise<Object>} Proration breakdown
   */
  async calculateProration(params) {
    const {
      subscriptionId,
      changeDate,
      newPriceCents,
      oldPriceCents,
      newQuantity = 1,
      oldQuantity = 1,
      prorationBehavior = 'create_prorations',
      appId
    } = params;

    // Validate inputs
    if (!subscriptionId || !changeDate || newPriceCents === undefined || oldPriceCents === undefined) {
      throw new ValidationError('subscriptionId, changeDate, newPriceCents, and oldPriceCents are required');
    }

    if (newPriceCents < 0 || oldPriceCents < 0 || newQuantity < 0 || oldQuantity < 0) {
      throw new ValidationError('Prices and quantities must be non-negative');
    }

    // 1. Fetch subscription to get billing period
    const subscription = await billingPrisma.billing_subscriptions.findUnique({
      where: { id: subscriptionId },
      include: { billing_customers: true }
    });

    if (!subscription) {
      throw new NotFoundError(`Subscription ${subscriptionId} not found`);
    }

    if (appId && subscription.billing_customers?.app_id !== appId) {
      throw new NotFoundError(`Subscription ${subscriptionId} not found`);
    }

    // 2. Calculate time-based proration
    const timeProration = ProrationCalculator.calculateTimeProration(
      changeDate,
      subscription.current_period_end,
      subscription.current_period_start
    );

    // 3. Calculate old plan credit (unused portion)
    const oldAmount = oldPriceCents * oldQuantity;
    const creditAmountCents = ProrationCalculator.roundToFinancialStandard(oldAmount * timeProration.prorationFactor);

    // 4. Calculate new plan charge (remaining period)
    const newAmount = newPriceCents * newQuantity;
    const chargeAmountCents = ProrationCalculator.roundToFinancialStandard(newAmount * timeProration.prorationFactor);

    // 5. Calculate net change
    const netAmountCents = chargeAmountCents - creditAmountCents;

    // 6. Build proration breakdown
    return {
      subscription_id: subscriptionId,
      change_date: changeDate,
      proration_behavior: prorationBehavior,
      time_proration: timeProration,
      old_plan: {
        price_cents: oldPriceCents,
        quantity: oldQuantity,
        total_cents: oldAmount,
        credit_cents: creditAmountCents
      },
      new_plan: {
        price_cents: newPriceCents,
        quantity: newQuantity,
        total_cents: newAmount,
        charge_cents: chargeAmountCents
      },
      net_change: {
        amount_cents: netAmountCents,
        type: netAmountCents >= 0 ? 'charge' : 'credit',
        description: netAmountCents >= 0
          ? `Prorated charge for upgrade`
          : `Prorated credit for downgrade`
      }
    };
  }

  /**
   * Execute mid-cycle subscription change with proration
   * @param {number} subscriptionId
   * @param {Object} changeDetails - New plan, price, quantity
   * @param {Object} options - Proration behavior, effective date
   * @param {string} appId - App ID for multi-tenant scoping
   * @returns {Promise<Object>} Updated subscription and proration charges
   */
  async applySubscriptionChange(subscriptionId, changeDetails, options = {}, appId) {
    // Validate inputs
    if (!subscriptionId || !changeDetails) {
      throw new ValidationError('subscriptionId and changeDetails are required');
    }

    const {
      newPriceCents,
      oldPriceCents,
      newQuantity = 1,
      oldQuantity = 1,
      newPlanId,
      oldPlanId
    } = changeDetails;

    const {
      prorationBehavior = 'create_prorations',
      effectiveDate = new Date(),
      invoiceImmediately = false
    } = options;

    // 1. Fetch subscription
    const subscription = await billingPrisma.billing_subscriptions.findUnique({
      where: { id: subscriptionId },
      include: { billing_customers: true }
    });

    if (!subscription) {
      throw new NotFoundError(`Subscription ${subscriptionId} not found`);
    }

    if (appId && subscription.billing_customers?.app_id !== appId) {
      throw new NotFoundError(`Subscription ${subscriptionId} not found`);
    }

    // 2. Calculate proration
    const proration = await this.calculateProration({
      subscriptionId,
      changeDate: effectiveDate,
      newPriceCents,
      oldPriceCents,
      newQuantity,
      oldQuantity,
      prorationBehavior,
      appId
    });

    // 3. If proration behavior is 'none', just update subscription without proration
    if (prorationBehavior === 'none') {
      logger.info('Proration behavior is "none", updating subscription without proration', {
        subscriptionId,
        changeDetails
      });

      const updatedSubscription = await ProrationExecutor.updateSubscription(
        subscription, changeDetails, null, { effectiveDate }
      );

      return {
        subscription: updatedSubscription,
        proration: null,
        charges: []
      };
    }

    // 4-6. Apply charges, update subscription, and record audit event atomically
    const { charges, updatedSubscription } = await billingPrisma.$transaction(async (tx) => {
      const charges = await ProrationExecutor.applyCharges(
        subscription, proration, changeDetails, { effectiveDate, prorationBehavior }, tx
      );

      const updatedSubscription = await ProrationExecutor.updateSubscription(
        subscription, changeDetails, proration, { effectiveDate }, tx
      );

      await ProrationExecutor.recordAuditEvent(
        subscription, proration,
        { oldPlanId, newPlanId, oldPriceCents, newPriceCents, oldQuantity, newQuantity },
        charges, { effectiveDate }, tx
      );

      return { charges, updatedSubscription };
    });

    return {
      subscription: updatedSubscription,
      proration,
      charges
    };
  }

  /**
   * Calculate refund for subscription cancellation
   * @param {number} subscriptionId
   * @param {Date} cancellationDate
   * @param {string} refundBehavior - 'partial_refund' | 'account_credit' | 'none'
   * @param {string} appId - App ID for multi-tenant scoping
   * @returns {Promise<Object>} Refund or credit details
   */
  async calculateCancellationRefund(subscriptionId, cancellationDate, refundBehavior = 'partial_refund', appId) {
    // Validate inputs
    if (!subscriptionId || !cancellationDate) {
      throw new ValidationError('subscriptionId and cancellationDate are required');
    }

    // 1. Fetch subscription
    const subscription = await billingPrisma.billing_subscriptions.findUnique({
      where: { id: subscriptionId },
      include: { billing_customers: true }
    });

    if (!subscription) {
      throw new NotFoundError(`Subscription ${subscriptionId} not found`);
    }

    if (appId && subscription.billing_customers?.app_id !== appId) {
      throw new NotFoundError(`Subscription ${subscriptionId} not found`);
    }

    // 2. Calculate time-based proration
    const timeProration = ProrationCalculator.calculateTimeProration(
      cancellationDate,
      subscription.current_period_end,
      subscription.current_period_start
    );

    // 3. Calculate refund amount
    const totalPaid = subscription.price_cents;
    const refundAmountCents = ProrationCalculator.roundToFinancialStandard(totalPaid * timeProration.prorationFactor);

    // 4. Handle different refund behaviors
    let action = 'none';
    let description = 'No refund issued';

    if (refundBehavior === 'partial_refund' && refundAmountCents > 0) {
      action = 'refund';
      description = `Partial refund of $${(refundAmountCents / 100).toFixed(2)} for unused service`;
    } else if (refundBehavior === 'account_credit' && refundAmountCents > 0) {
      action = 'account_credit';
      description = `Account credit of $${(refundAmountCents / 100).toFixed(2)} for unused service`;
    }

    return {
      subscription_id: subscriptionId,
      cancellation_date: cancellationDate,
      refund_behavior: refundBehavior,
      time_proration: timeProration,
      total_paid_cents: totalPaid,
      refund_amount_cents: refundAmountCents,
      action,
      description
    };
  }

}

module.exports = ProrationService;
