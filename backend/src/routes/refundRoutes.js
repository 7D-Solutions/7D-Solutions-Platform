const express = require('express');
const BillingService = require('../billingService');
const { requireAppId, rejectSensitiveData } = require('../middleware');
const { createRefundValidator } = require('../validators/refundValidators');

const router = express.Router();
const billingService = new BillingService();

// Apply requireAppId middleware to all routes in this file
router.use(requireAppId());

/**
 * POST /refunds?app_id=X
 * Create a refund for a charge
 *
 * Headers:
 *   Idempotency-Key: <uuid> (REQUIRED)
 *
 * Body:
 *   {
 *     charge_id: number (local billing_charges.id),
 *     amount_cents: number,
 *     currency: string (default: 'usd'),
 *     reason: string (optional),
 *     reference_id: string (unique per app, REQUIRED),
 *     note: string (optional),
 *     metadata: object (optional)
 *   }
 *
 * Responses:
 *   201: { refund: {...} }
 *   400: Missing app_id, Idempotency-Key, or required fields
 *   404: Charge not found (or belongs to different app_id)
 *   409: Charge not settled in processor OR Idempotency-Key reuse with different payload
 *   502: Tilled refund creation failed
 */
router.post('/', rejectSensitiveData, createRefundValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const idempotencyKey = req.headers['idempotency-key'];

    const {
      charge_id,
      amount_cents,
      currency,
      reason,
      reference_id,
      note,
      metadata,
    } = req.body;

    // Compute request hash for idempotency
    const requestHash = billingService.computeRequestHash(
      'POST',
      '/refunds',
      req.body
    );

    // Check for idempotent response
    const cachedResponse = await billingService.getIdempotentResponse(
      appId,
      idempotencyKey,
      requestHash
    );

    if (cachedResponse) {
      return res.status(cachedResponse.statusCode).json(cachedResponse.body);
    }

    // Create refund
    const refund = await billingService.createRefund(
      appId,
      {
        chargeId: charge_id,
        amountCents: amount_cents,
        currency,
        reason,
        referenceId: reference_id,
        note,
        metadata,
      },
      { idempotencyKey, requestHash }
    );

    const responseBody = { refund };
    const statusCode = 201;

    // Store idempotent response
    await billingService.storeIdempotentResponse(
      appId,
      idempotencyKey,
      requestHash,
      statusCode,
      responseBody
    );

    res.status(statusCode).json(responseBody);
  } catch (error) {
    next(error);
  }
});

module.exports = router;