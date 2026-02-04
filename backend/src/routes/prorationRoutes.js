const express = require('express');
const BillingService = require('../billingService');
const { requireAppId, rejectSensitiveData } = require('../middleware');
const {
  calculateProrationValidator,
  applySubscriptionChangeValidator,
  calculateCancellationRefundValidator
} = require('../validators/prorationValidators');

const router = express.Router();
const billingService = new BillingService();

// Apply requireAppId middleware to all routes in this file
router.use(requireAppId());

/**
 * POST /proration/calculate
 * Calculate proration preview for subscription change
 *
 * Body:
 *   {
 *     subscription_id: number,
 *     change_date: string (ISO 8601),
 *     new_price_cents: number,
 *     old_price_cents: number,
 *     new_quantity: number (optional, default: 1),
 *     old_quantity: number (optional, default: 1),
 *     proration_behavior: string (optional, 'create_prorations'|'none'|'always_invoice')
 *   }
 *
 * Response:
 *   {
 *     proration: { ... } // Proration breakdown from ProrationService.calculateProration
 *   }
 */
router.post('/calculate', rejectSensitiveData, calculateProrationValidator, async (req, res, next) => {
  try {
    const {
      subscription_id,
      change_date,
      new_price_cents,
      old_price_cents,
      new_quantity = 1,
      old_quantity = 1,
      proration_behavior = 'create_prorations'
    } = req.body;

    const proration = await billingService.calculateProration({
      subscriptionId: subscription_id,
      changeDate: new Date(change_date),
      newPriceCents: new_price_cents,
      oldPriceCents: old_price_cents,
      newQuantity: new_quantity,
      oldQuantity: old_quantity,
      prorationBehavior: proration_behavior
    });

    res.json({ proration });
  } catch (error) {
    next(error);
  }
});

module.exports = router;