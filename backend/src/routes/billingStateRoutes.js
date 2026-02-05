const express = require('express');
const BillingStateService = require('../services/BillingStateService');
const { requireAppId } = require('../middleware');
const { getBillingStateValidator } = require('../validators/billingStateValidators');

const router = express.Router();
const billingStateService = new BillingStateService();

// GET /api/billing/state (billing snapshot + entitlements)
router.get('/', requireAppId(), getBillingStateValidator, async (req, res, next) => {
  try {
    const { external_customer_id } = req.query;
    const appId = req.verifiedAppId;

    const state = await billingStateService.getBillingState(appId, external_customer_id);
    res.json(state);
  } catch (error) {
    next(error);
  }
});

module.exports = router;