const express = require('express');
const BillingService = require('../billingService');
const { requireAppId } = require('../middleware');
const { getBillingStateValidator } = require('../validators/billingStateValidators');

const router = express.Router();
const billingService = new BillingService();

// GET /api/billing/state (billing snapshot + entitlements)
router.get('/', requireAppId(), getBillingStateValidator, async (req, res, next) => {
  try {
    const { external_customer_id } = req.query;
    const appId = req.verifiedAppId;

    const state = await billingService.getBillingState(appId, external_customer_id);
    res.json(state);
  } catch (error) {
    next(error);
  }
});

module.exports = router;