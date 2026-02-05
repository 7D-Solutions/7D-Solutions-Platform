const { billingPrisma } = require('../../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');

/**
 * ProrationExecutor - Handles execution side-effects of proration changes
 *
 * Static methods for charge record creation, subscription updates,
 * and audit event recording. Dependencies passed via parameters.
 */
class ProrationExecutor {
  /**
   * Create proration credit/debit charge records
   * @param {Object} subscription - The billing subscription record (with billing_customers)
   * @param {Object} proration - Proration calculation result from calculateProration
   * @param {Object} changeDetails - { oldPlanId, newPlanId, oldPriceCents, newPriceCents }
   * @param {Object} options - { effectiveDate, prorationBehavior }
   * @returns {Promise<Array>} Created charge records
   */
  static async applyCharges(subscription, proration, changeDetails, options, tx = billingPrisma) {
    const { oldPlanId, newPlanId, oldPriceCents, newPriceCents } = changeDetails;
    const { effectiveDate, prorationBehavior } = options;
    const charges = [];

    // Create credit for old plan (if any)
    if (proration.old_plan.credit_cents > 0) {
      const creditCharge = await tx.billing_charges.create({
        data: {
          app_id: subscription.billing_customers.app_id,
          billing_customer_id: subscription.billing_customer_id,
          charge_type: 'proration_credit',
          amount_cents: -proration.old_plan.credit_cents, // Negative for credit
          status: 'pending',
          reason: 'mid_cycle_downgrade',
          reference_id: `proration_sub_${subscription.id}_${effectiveDate.toISOString().split('T')[0]}_credit`,
          metadata: {
            proration: {
              subscription_id: subscription.id,
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
      const chargeCharge = await tx.billing_charges.create({
        data: {
          app_id: subscription.billing_customers.app_id,
          billing_customer_id: subscription.billing_customer_id,
          charge_type: 'proration_charge',
          amount_cents: proration.new_plan.charge_cents, // Positive for charge
          status: 'pending',
          reason: 'mid_cycle_upgrade',
          reference_id: `proration_sub_${subscription.id}_${effectiveDate.toISOString().split('T')[0]}_charge`,
          metadata: {
            proration: {
              subscription_id: subscription.id,
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

    return charges;
  }

  /**
   * Update subscription with new plan details and proration metadata
   * @param {Object} subscription - Current subscription record
   * @param {Object} changeDetails - { newPlanId, newPriceCents }
   * @param {Object} proration - Proration calculation result (null if behavior='none')
   * @param {Object} options - { effectiveDate }
   * @returns {Promise<Object>} Updated subscription record
   */
  static async updateSubscription(subscription, changeDetails, proration, options, tx = billingPrisma) {
    const { newPlanId, newPriceCents } = changeDetails;
    const { effectiveDate } = options;
    const prorationApplied = proration !== null;

    return tx.billing_subscriptions.update({
      where: { id: subscription.id },
      data: {
        plan_id: newPlanId || undefined,
        price_cents: newPriceCents,
        metadata: {
          ...(subscription.metadata || {}),
          last_change: {
            date: effectiveDate,
            type: 'plan_change',
            proration_applied: prorationApplied,
            ...(prorationApplied && { proration_net_amount_cents: proration.net_change.amount_cents })
          }
        },
        updated_at: new Date()
      }
    });
  }

  /**
   * Create billing_events audit record for the proration
   * @param {Object} subscription - The billing subscription (with billing_customers)
   * @param {Object} proration - Proration calculation result
   * @param {Object} changeDetails - { oldPlanId, newPlanId, oldPriceCents, newPriceCents, oldQuantity, newQuantity }
   * @param {Array} charges - Created charge records
   * @param {Object} options - { effectiveDate }
   * @returns {Promise<Object>} Created event record
   */
  static async recordAuditEvent(subscription, proration, changeDetails, charges, options, tx = billingPrisma) {
    const { oldPlanId, newPlanId, oldPriceCents, newPriceCents, oldQuantity, newQuantity } = changeDetails;
    const { effectiveDate } = options;

    return tx.billing_events.create({
      data: {
        app_id: subscription.billing_customers.app_id,
        event_type: 'proration_applied',
        source: 'proration_service',
        entity_type: 'subscription',
        entity_id: subscription.id.toString(),
        payload: {
          subscription_id: subscription.id,
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
  }
}

module.exports = ProrationExecutor;
