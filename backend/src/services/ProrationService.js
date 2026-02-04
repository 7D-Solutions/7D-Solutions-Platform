const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError } = require('../utils/errors');

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
      prorationBehavior = 'create_prorations'
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

    // 2. Calculate time-based proration
    const timeProration = this.calculateTimeProration(
      changeDate,
      subscription.current_period_end,
      subscription.current_period_start
    );

    // 3. Calculate old plan credit (unused portion)
    const oldAmount = oldPriceCents * oldQuantity;
    const creditAmountCents = this.roundToFinancialStandard(oldAmount * timeProration.prorationFactor);

    // 4. Calculate new plan charge (remaining period)
    const newAmount = newPriceCents * newQuantity;
    const chargeAmountCents = this.roundToFinancialStandard(newAmount * timeProration.prorationFactor);

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
   * @returns {Promise<Object>} Updated subscription and proration charges
   */
  async applySubscriptionChange(subscriptionId, changeDetails, options = {}) {
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

    // 2. Calculate proration
    const proration = await this.calculateProration({
      subscriptionId,
      changeDate: effectiveDate,
      newPriceCents,
      oldPriceCents,
      newQuantity,
      oldQuantity,
      prorationBehavior
    });

    // 3. If proration behavior is 'none', just update subscription without proration
    if (prorationBehavior === 'none') {
      logger.info('Proration behavior is "none", updating subscription without proration', {
        subscriptionId,
        changeDetails
      });

      // Update subscription in database
      const updatedSubscription = await billingPrisma.billing_subscriptions.update({
        where: { id: subscriptionId },
        data: {
          plan_id: newPlanId || undefined,
          price_cents: newPriceCents,
          metadata: {
            ...(subscription.metadata || {}),
            last_change: {
              date: effectiveDate,
              type: 'plan_change',
              proration_applied: false
            }
          },
          updated_at: new Date()
        }
      });

      return {
        subscription: updatedSubscription,
        proration: null,
        charges: []
      };
    }

    // 4. Create proration charge/credit records
    const charges = [];

    // Create credit for old plan (if any)
    if (proration.old_plan.credit_cents > 0) {
      const creditCharge = await billingPrisma.billing_charges.create({
        data: {
          app_id: subscription.billing_customers.app_id,
          billing_customer_id: subscription.billing_customer_id,
          charge_type: 'proration_credit',
          amount_cents: -proration.old_plan.credit_cents, // Negative for credit
          status: 'pending',
          reason: 'mid_cycle_downgrade',
          reference_id: `proration_sub_${subscriptionId}_${effectiveDate.toISOString().split('T')[0]}_credit`,
          metadata: {
            proration: {
              subscription_id: subscriptionId,
              change_date: effectiveDate,
              old_plan_id: oldPlanId,
              new_plan_id: newPlanId,
              old_price_cents: oldPriceCents,
              new_price_cents: newPriceCents,
              days_used: proration.time_proration.daysUsed,
              days_remaining: proration.time_proration.daysRemaining,
              days_total: proration.time_proration.daysTotal,
              proration_factor: proration.time_proration.prorationFactor,
              behavior: prorationBehavior
            }
          },
          created_at: new Date()
        }
      });
      charges.push(creditCharge);
    }

    // Create charge for new plan (if any)
    if (proration.new_plan.charge_cents > 0) {
      const chargeCharge = await billingPrisma.billing_charges.create({
        data: {
          app_id: subscription.billing_customers.app_id,
          billing_customer_id: subscription.billing_customer_id,
          charge_type: 'proration_charge',
          amount_cents: proration.new_plan.charge_cents, // Positive for charge
          status: 'pending',
          reason: 'mid_cycle_upgrade',
          reference_id: `proration_sub_${subscriptionId}_${effectiveDate.toISOString().split('T')[0]}_charge`,
          metadata: {
            proration: {
              subscription_id: subscriptionId,
              change_date: effectiveDate,
              old_plan_id: oldPlanId,
              new_plan_id: newPlanId,
              old_price_cents: oldPriceCents,
              new_price_cents: newPriceCents,
              days_used: proration.time_proration.daysUsed,
              days_remaining: proration.time_proration.daysRemaining,
              days_total: proration.time_proration.daysTotal,
              proration_factor: proration.time_proration.prorationFactor,
              behavior: prorationBehavior
            }
          },
          created_at: new Date()
        }
      });
      charges.push(chargeCharge);
    }

    // 5. Update subscription
    const updatedSubscription = await billingPrisma.billing_subscriptions.update({
      where: { id: subscriptionId },
      data: {
        plan_id: newPlanId || undefined,
        price_cents: newPriceCents,
        metadata: {
          ...(subscription.metadata || {}),
          last_change: {
            date: effectiveDate,
            type: 'plan_change',
            proration_applied: true,
            proration_net_amount_cents: proration.net_change.amount_cents
          }
        },
        updated_at: new Date()
      }
    });

    // 6. Create audit event
    await billingPrisma.billing_events.create({
      data: {
        app_id: subscription.billing_customers.app_id,
        event_type: 'proration_applied',
        source: 'proration_service',
        entity_type: 'subscription',
        entity_id: subscriptionId.toString(),
        payload: {
          subscription_id: subscriptionId,
          change_type: proration.net_change.amount_cents >= 0 ? 'plan_upgrade' : 'plan_downgrade',
          old_plan: { plan_id: oldPlanId, price_cents: oldPriceCents, quantity: oldQuantity },
          new_plan: { plan_id: newPlanId, price_cents: newPriceCents, quantity: newQuantity },
          proration_breakdown: {
            credit_amount_cents: proration.old_plan.credit_cents,
            charge_amount_cents: proration.new_plan.charge_cents,
            net_amount_cents: proration.net_change.amount_cents
          },
          effective_date: effectiveDate,
          charges_created: charges.map(c => ({ id: c.id, type: c.charge_type, amount_cents: c.amount_cents }))
        },
        created_at: new Date()
      }
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
   * @returns {Promise<Object>} Refund or credit details
   */
  async calculateCancellationRefund(subscriptionId, cancellationDate, refundBehavior = 'partial_refund') {
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

    // 2. Calculate time-based proration
    const timeProration = this.calculateTimeProration(
      cancellationDate,
      subscription.current_period_end,
      subscription.current_period_start
    );

    // 3. Calculate refund amount
    const totalPaid = subscription.price_cents;
    const refundAmountCents = this.roundToFinancialStandard(totalPaid * timeProration.prorationFactor);

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

  /**
   * Calculate days remaining in billing period
   * @param {Date} changeDate
   * @param {Date} periodEnd
   * @param {Date} periodStart
   * @returns {Object} { daysUsed, daysRemaining, daysTotal, prorationFactor }
   */
  calculateTimeProration(changeDate, periodEnd, periodStart) {
    // Normalize all dates to midnight UTC for consistency
    const change = new Date(changeDate);
    change.setUTCHours(0, 0, 0, 0);

    const start = new Date(periodStart);
    start.setUTCHours(0, 0, 0, 0);

    const end = new Date(periodEnd);
    end.setUTCHours(0, 0, 0, 0);

    // Edge case: change at period start
    if (change.getTime() === start.getTime()) {
      return {
        daysUsed: 0,
        daysRemaining: Math.round((end - start) / (24 * 60 * 60 * 1000)),
        daysTotal: Math.round((end - start) / (24 * 60 * 60 * 1000)),
        prorationFactor: 1.0,
        note: 'change_at_period_start'
      };
    }

    // Edge case: change at or after period end
    if (change.getTime() >= end.getTime()) {
      return {
        daysUsed: Math.round((end - start) / (24 * 60 * 60 * 1000)),
        daysRemaining: 0,
        daysTotal: Math.round((end - start) / (24 * 60 * 60 * 1000)),
        prorationFactor: 0.0,
        note: 'change_at_period_end'
      };
    }

    // Calculate milliseconds
    const totalMs = end - start;
    const usedMs = change - start;
    const remainingMs = end - change;

    // Convert to days
    const msPerDay = 24 * 60 * 60 * 1000;
    const daysTotal = totalMs / msPerDay;
    const daysUsed = usedMs / msPerDay;
    const daysRemaining = remainingMs / msPerDay;

    // Calculate proration factor (4 decimal places)
    const prorationFactor = daysRemaining / daysTotal;

    return {
      daysUsed: Math.floor(daysUsed),
      daysRemaining: Math.ceil(daysRemaining),
      daysTotal: Math.round(daysTotal),
      prorationFactor: Math.round(prorationFactor * 10000) / 10000
    };
  }

  /**
   * Apply financial rounding to proration amounts
   * @param {number} amountCents
   * @returns {number} Rounded amount
   */
  roundToFinancialStandard(amountCents) {
    // Round half-up to nearest cent
    return Math.round(amountCents);
  }

  /**
   * Helper: Normalize date to UTC midnight
   * @param {Date} date
   * @returns {Date} Normalized date
   */
  normalizeToUTCMidnight(date) {
    const d = new Date(date);
    d.setUTCHours(0, 0, 0, 0);
    return d;
  }
}

module.exports = ProrationService;