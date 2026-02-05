const express = require('express');
const { getTilledClient } = require('../tilledClientFactory');
const SubscriptionService = require('../services/SubscriptionService');
const ProrationService = require('../services/ProrationService');
const { requireAppId, rejectSensitiveData } = require('../middleware');
const {
  getSubscriptionByIdValidator,
  listSubscriptionsValidator,
  createSubscriptionValidator,
  cancelSubscriptionValidator,
  updateSubscriptionValidator,
  changeCycleValidator
} = require('../validators/subscriptionValidators');
const {
  applySubscriptionChangeValidator,
  calculateCancellationRefundValidator
} = require('../validators/prorationValidators');

const router = express.Router();
const subscriptionService = new SubscriptionService(getTilledClient);
const prorationService = new ProrationService(getTilledClient);

// Apply requireAppId middleware to all routes in this file
router.use(requireAppId());

// GET /api/billing/subscriptions/:id
router.get('/:id', getSubscriptionByIdValidator, async (req, res, next) => {
  try {
    const { id } = req.params;
    const appId = req.verifiedAppId;

    const subscription = await subscriptionService.getSubscriptionById(appId, Number(id));
    res.json(subscription);
  } catch (error) {
    next(error);
  }
});

// GET /api/billing/subscriptions (with filters)
router.get('/', listSubscriptionsValidator, async (req, res, next) => {
  try {
    const { billing_customer_id, status } = req.query;
    const appId = req.verifiedAppId;

    const filters = { appId };
    if (billing_customer_id) filters.billingCustomerId = Number(billing_customer_id);
    if (status) filters.status = status;

    const subscriptions = await subscriptionService.listSubscriptions(filters);
    res.json(subscriptions);
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/subscriptions
router.post('/', rejectSensitiveData, createSubscriptionValidator, async (req, res, next) => {
  try {
    const {
      billing_customer_id,
      payment_method_id,
      plan_id,
      plan_name,
      price_cents,
      interval_unit,
      interval_count,
      billing_cycle_anchor,
      trial_end,
      cancel_at_period_end,
      metadata
    } = req.body;
    const appId = req.verifiedAppId;

    const subscription = await subscriptionService.createSubscription(
      appId,
      billing_customer_id,
      payment_method_id,
      plan_id,
      plan_name,
      price_cents,
      {
        intervalUnit: interval_unit,
        intervalCount: interval_count,
        billingCycleAnchor: billing_cycle_anchor,
        trialEnd: trial_end,
        cancelAtPeriodEnd: cancel_at_period_end,
        metadata
      }
    );

    res.status(201).json(subscription);
  } catch (error) {
    next(error);
  }
});

// DELETE /api/billing/subscriptions/:id
router.delete('/:id', cancelSubscriptionValidator, async (req, res, next) => {
  try {
    const { id } = req.params;
    const { at_period_end } = req.query;
    const appId = req.verifiedAppId;

    const subscription = await subscriptionService.cancelSubscriptionEx(
      appId,
      Number(id),
      { atPeriodEnd: at_period_end === 'true' }
    );
    res.json(subscription);
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/subscriptions/change-cycle
router.post('/change-cycle', rejectSensitiveData, changeCycleValidator, async (req, res, next) => {
  try {
    const { ...payload } = req.body;
    const appId = req.verifiedAppId;

    const result = await subscriptionService.changeCycle(appId, payload);
    res.status(201).json(result);
  } catch (error) {
    next(error);
  }
});

// PUT /api/billing/subscriptions/:id
// NOTE: price_cents changes affect FUTURE billing cycles, not immediate proration
// Tilled does not support changing billing cycles (interval_unit, interval_count, billing_cycle_anchor)
// For cycle changes, use cancel+create pattern
router.put('/:id', rejectSensitiveData, updateSubscriptionValidator, async (req, res, next) => {
  try {
    const { id } = req.params;
    const { app_id, ...updates } = req.body; // Exclude app_id from updates
    const appId = req.verifiedAppId;

    const subscription = await subscriptionService.updateSubscription(appId, Number(id), updates);
    res.json(subscription);
  } catch (error) {
    next(error);
  }
});

/**
 * POST /subscriptions/:subscription_id/proration/apply
 * Apply subscription change with proration
 *
 * Body:
 *   {
 *     new_price_cents: number,
 *     old_price_cents: number,
 *     new_quantity: number (optional, default: 1),
 *     old_quantity: number (optional, default: 1),
 *     new_plan_id: string (optional),
 *     old_plan_id: string (optional),
 *     proration_behavior: string (optional, 'create_prorations'|'none'|'always_invoice'),
 *     effective_date: string (ISO 8601, optional, default: now),
 *     invoice_immediately: boolean (optional, default: false)
 *   }
 *
 * Response:
 *   {
 *     subscription: { ... }, // Updated subscription
 *     proration: { ... }, // Proration breakdown
 *     charges: [ ... ] // Created proration charges/credits
 *   }
 */
router.post('/:subscription_id/proration/apply', rejectSensitiveData, applySubscriptionChangeValidator, async (req, res, next) => {
  try {
    const { subscription_id } = req.params;
    const {
      new_price_cents,
      old_price_cents,
      new_quantity = 1,
      old_quantity = 1,
      new_plan_id,
      old_plan_id,
      proration_behavior = 'create_prorations',
      effective_date,
      invoice_immediately = false
    } = req.body;

    const changeDetails = {
      newPriceCents: new_price_cents,
      oldPriceCents: old_price_cents,
      newQuantity: new_quantity,
      oldQuantity: old_quantity,
      newPlanId: new_plan_id,
      oldPlanId: old_plan_id
    };

    const options = {};
    if (proration_behavior) options.prorationBehavior = proration_behavior;
    if (effective_date) options.effectiveDate = new Date(effective_date);
    if (invoice_immediately !== undefined) options.invoiceImmediately = invoice_immediately;

    const result = await prorationService.applySubscriptionChange(
      Number(subscription_id),
      changeDetails,
      options,
      req.verifiedAppId
    );

    res.status(200).json({
      subscription: result.subscription,
      proration: result.proration,
      charges: result.charges
    });
  } catch (error) {
    next(error);
  }
});

/**
 * POST /subscriptions/:subscription_id/proration/cancellation-refund
 * Calculate refund for subscription cancellation
 *
 * Body:
 *   {
 *     cancellation_date: string (ISO 8601),
 *     refund_behavior: string (optional, 'partial_refund'|'account_credit'|'none')
 *   }
 *
 * Response:
 *   {
 *     cancellation_refund: { ... } // Refund details from ProrationService.calculateCancellationRefund
 *   }
 */
router.post('/:subscription_id/proration/cancellation-refund', rejectSensitiveData, calculateCancellationRefundValidator, async (req, res, next) => {
  try {
    const { subscription_id } = req.params;
    const { cancellation_date, refund_behavior = 'partial_refund' } = req.body;

    const cancellationRefund = await prorationService.calculateCancellationRefund(
      Number(subscription_id),
      new Date(cancellation_date),
      refund_behavior,
      req.verifiedAppId
    );

    res.json({ cancellation_refund: cancellationRefund });
  } catch (error) {
    next(error);
  }
});

module.exports = router;