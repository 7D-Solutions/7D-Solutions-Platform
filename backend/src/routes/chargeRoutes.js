const express = require('express');
const BillingService = require('../billingService');
const { requireAppId, rejectSensitiveData } = require('../middleware');
const { createOneTimeChargeValidator } = require('../validators/chargeValidators');

const router = express.Router();
const billingService = new BillingService();

// Apply requireAppId middleware to all routes in this file
router.use(requireAppId());

/**
 * POST /charges/one-time?app_id=X
 * Create a one-time charge for operational add-ons (extra pickup, tip, etc.)
 *
 * Headers:
 *   Idempotency-Key: <uuid> (REQUIRED)
 *
 * Body:
 *   {
 *     external_customer_id: string,
 *     amount_cents: number,
 *     currency: string (default: 'usd'),
 *     reason: string ('extra_pickup', 'tip', etc.),
 *     reference_id: string (unique per app),
 *     service_date: string (ISO date, optional),
 *     note: string (optional),
 *     metadata: object (optional)
 *   }
 *
 * Responses:
 *   201: { charge: {...} }
 *   400: Missing app_id, Idempotency-Key, or required fields
 *   404: Customer not found
 *   409: No default payment method OR duplicate reference_id
 *   502: Tilled charge creation failed
 */
router.post('/one-time', rejectSensitiveData, createOneTimeChargeValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const idempotencyKey = req.headers['idempotency-key'];

    const {
      external_customer_id,
      amount_cents,
      currency,
      reason,
      reference_id,
      service_date,
      note,
      metadata,
    } = req.body;

    // Compute request hash for idempotency
    const requestHash = billingService.computeRequestHash(
      'POST',
      '/charges/one-time',
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

    // Create one-time charge
    const charge = await billingService.createOneTimeCharge(
      appId,
      {
        externalCustomerId: external_customer_id,
        amountCents: amount_cents,
        currency,
        reason,
        referenceId: reference_id,
        serviceDate: service_date,
        note,
        metadata,
      },
      { idempotencyKey, requestHash }
    );

    const responseBody = { charge };
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