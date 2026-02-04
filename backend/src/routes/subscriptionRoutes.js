const express = require('express');
const BillingService = require('../billingService');
const { requireAppId, rejectSensitiveData } = require('../middleware');
const {
  getSubscriptionByIdValidator,
  listSubscriptionsValidator,
  createSubscriptionValidator,
  cancelSubscriptionValidator,
  updateSubscriptionValidator
} = require('../validators/subscriptionValidators');

const router = express.Router();
const billingService = new BillingService();

// Apply requireAppId middleware to all routes in this file
router.use(requireAppId());

// GET /api/billing/subscriptions/:id
router.get('/:id', getSubscriptionByIdValidator, async (req, res, next) => {
  try {
    const { id } = req.params;
    const appId = req.verifiedAppId;

    const subscription = await billingService.getSubscriptionById(appId, Number(id));
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

    const subscriptions = await billingService.listSubscriptions(filters);
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

    const subscription = await billingService.createSubscription(
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

    const subscription = await billingService.cancelSubscriptionEx(
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
router.post('/change-cycle', rejectSensitiveData, async (req, res, next) => {
  try {
    const { ...payload } = req.body;
    const appId = req.verifiedAppId;

    const result = await billingService.changeCycle(appId, payload);
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

    const subscription = await billingService.updateSubscription(appId, Number(id), updates);
    res.json(subscription);
  } catch (error) {
    next(error);
  }
});

module.exports = router;