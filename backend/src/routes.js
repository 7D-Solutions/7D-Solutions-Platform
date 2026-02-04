const express = require('express');
const BillingService = require('./billingService');
const logger = require('@fireproof/infrastructure/utils/logger');
const { billingPrisma } = require('./prisma');
const { requireAppId, rejectSensitiveData } = require('./middleware');
const {
  getCustomerByIdValidator,
  getCustomerByExternalIdValidator,
  createCustomerValidator,
  setDefaultPaymentMethodValidator,
  updateCustomerValidator,
  getBillingStateValidator,
  listPaymentMethodsValidator,
  addPaymentMethodValidator,
  setDefaultPaymentMethodByIdValidator,
  deletePaymentMethodValidator,
  getSubscriptionByIdValidator,
  listSubscriptionsValidator,
  createSubscriptionValidator,
  cancelSubscriptionValidator,
  updateSubscriptionValidator,
  createOneTimeChargeValidator,
  createRefundValidator,
  getTaxRatesByJurisdictionValidator,
  createTaxRateValidator,
  createTaxExemptionValidator,
  getTaxCalculationsForInvoiceValidator,
  calculateProrationValidator,
  applySubscriptionChangeValidator,
  calculateCancellationRefundValidator,
  recordUsageValidator,
  calculateUsageChargesValidator,
  getUsageReportValidator
} = require('./validators/requestValidators');

const router = express.Router();
const billingService = new BillingService();

// GET /api/billing/health (admin-only health check)
router.get('/health', requireAppId(), async (req, res) => {
  const appId = req.verifiedAppId;

  const checks = {
    timestamp: new Date().toISOString(),
    app_id: appId,
    database: { status: 'unknown', error: null },
    tilled_config: { status: 'unknown', error: null }
  };

  // Check database connectivity
  try {
    await billingPrisma.$queryRaw`SELECT 1`;
    checks.database.status = 'healthy';
  } catch (error) {
    checks.database.status = 'unhealthy';
    checks.database.error = error.message;
  }

  // Check Tilled credentials present
  try {
    const prefix = appId.toUpperCase();
    const secretKey = process.env[`TILLED_SECRET_KEY_${prefix}`];
    const accountId = process.env[`TILLED_ACCOUNT_ID_${prefix}`];
    const webhookSecret = process.env[`TILLED_WEBHOOK_SECRET_${prefix}`];
    const sandbox = process.env.TILLED_SANDBOX;

    const missing = [];
    if (!secretKey) missing.push('TILLED_SECRET_KEY');
    if (!accountId) missing.push('TILLED_ACCOUNT_ID');
    if (!webhookSecret) missing.push('TILLED_WEBHOOK_SECRET');
    if (sandbox === undefined) missing.push('TILLED_SANDBOX');

    if (missing.length > 0) {
      checks.tilled_config.status = 'unhealthy';
      checks.tilled_config.error = `Missing credentials: ${missing.join(', ')}`;
    } else {
      checks.tilled_config.status = 'healthy';
      checks.tilled_config.sandbox_mode = sandbox === 'true';
    }
  } catch (error) {
    checks.tilled_config.status = 'unhealthy';
    checks.tilled_config.error = error.message;
  }

  // Overall health
  const allHealthy = checks.database.status === 'healthy' &&
                     checks.tilled_config.status === 'healthy';

  const statusCode = allHealthy ? 200 : 503;
  checks.overall_status = allHealthy ? 'healthy' : 'degraded';

  res.status(statusCode).json(checks);
});

// GET /api/billing/customers/:id
router.get('/customers/:id', requireAppId(), getCustomerByIdValidator, async (req, res, next) => {
  try {
    const { id } = req.params;
    const appId = req.verifiedAppId;

    const customer = await billingService.getCustomerById(appId, Number(id));
    res.json(customer);
  } catch (error) {
    next(error);
  }
});

// GET /api/billing/customers (by external_customer_id)
router.get('/customers', requireAppId(), getCustomerByExternalIdValidator, async (req, res, next) => {
  try {
    const { external_customer_id } = req.query;
    const appId = req.verifiedAppId;

    const customer = await billingService.findCustomer(appId, external_customer_id);
    res.json(customer);
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/customers
router.post('/customers', requireAppId(), rejectSensitiveData, createCustomerValidator, async (req, res, next) => {
  try {
    const { email, name, external_customer_id, metadata } = req.body;
    const appId = req.verifiedAppId;

    const customer = await billingService.createCustomer(appId, email, name, external_customer_id, metadata);
    res.status(201).json(customer);
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/customers/:id/default-payment-method
router.post('/customers/:id/default-payment-method', requireAppId(), rejectSensitiveData, setDefaultPaymentMethodValidator, async (req, res, next) => {
  try {
    const { id } = req.params;
    const { payment_method_id, payment_method_type } = req.body;
    const appId = req.verifiedAppId;

    const customer = await billingService.setDefaultPaymentMethod(
      appId,
      Number(id),
      payment_method_id,
      payment_method_type
    );
    res.json(customer);
  } catch (error) {
    next(error);
  }
});

// PUT /api/billing/customers/:id
router.put('/customers/:id', requireAppId(), rejectSensitiveData, updateCustomerValidator, async (req, res, next) => {
  try {
    const { id } = req.params;
    const { ...updates } = req.body;
    const appId = req.verifiedAppId;

    const customer = await billingService.updateCustomer(appId, Number(id), updates);
    res.json(customer);
  } catch (error) {
    next(error);
  }
});

// GET /api/billing/state (billing snapshot + entitlements)
router.get('/state', requireAppId(), getBillingStateValidator, async (req, res, next) => {
  try {
    const { external_customer_id } = req.query;
    const appId = req.verifiedAppId;

    const state = await billingService.getBillingState(appId, external_customer_id);
    res.json(state);
  } catch (error) {
    next(error);
  }
});

// GET /api/billing/payment-methods (list for customer)
router.get('/payment-methods', requireAppId(), listPaymentMethodsValidator, async (req, res, next) => {
  try {
    const { billing_customer_id } = req.query;
    const appId = req.verifiedAppId;

    const result = await billingService.listPaymentMethods(appId, Number(billing_customer_id));
    res.json(result);
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/payment-methods (add payment method)
router.post('/payment-methods', requireAppId(), rejectSensitiveData, addPaymentMethodValidator, async (req, res, next) => {
  try {
    const { billing_customer_id, payment_method_id } = req.body;
    const appId = req.verifiedAppId;

    const paymentMethod = await billingService.addPaymentMethod(
      appId,
      Number(billing_customer_id),
      payment_method_id
    );
    res.status(201).json(paymentMethod);
  } catch (error) {
    next(error);
  }
});

// PUT /api/billing/payment-methods/:id/default (set default payment method)
router.put('/payment-methods/:id/default', requireAppId(), rejectSensitiveData, setDefaultPaymentMethodByIdValidator, async (req, res, next) => {
  try {
    const { id: tilledPaymentMethodId } = req.params;
    const { billing_customer_id } = req.body;
    const appId = req.verifiedAppId;

    const paymentMethod = await billingService.setDefaultPaymentMethodById(
      appId,
      Number(billing_customer_id),
      tilledPaymentMethodId
    );
    res.json(paymentMethod);
  } catch (error) {
    next(error);
  }
});

// DELETE /api/billing/payment-methods/:id (soft delete payment method)
router.delete('/payment-methods/:id', requireAppId(), deletePaymentMethodValidator, async (req, res, next) => {
  try {
    const { id: tilledPaymentMethodId } = req.params;
    const { billing_customer_id } = req.query;
    const appId = req.verifiedAppId;

    const result = await billingService.deletePaymentMethod(
      appId,
      Number(billing_customer_id),
      tilledPaymentMethodId
    );
    res.json(result);
  } catch (error) {
    next(error);
  }
});

// GET /api/billing/subscriptions/:id
router.get('/subscriptions/:id', requireAppId(), getSubscriptionByIdValidator, async (req, res, next) => {
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
router.get('/subscriptions', requireAppId(), listSubscriptionsValidator, async (req, res, next) => {
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
router.post('/subscriptions', requireAppId(), rejectSensitiveData, createSubscriptionValidator, async (req, res, next) => {
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
router.delete('/subscriptions/:id', requireAppId(), cancelSubscriptionValidator, async (req, res, next) => {
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
router.post('/subscriptions/change-cycle', requireAppId(), rejectSensitiveData, async (req, res, next) => {
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
router.put('/subscriptions/:id', requireAppId(), rejectSensitiveData, updateSubscriptionValidator, async (req, res, next) => {
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

// POST /api/billing/webhooks/:app_id (NO auth middleware - signature verification only)
router.post('/webhooks/:app_id', async (req, res, next) => {
  try {
    const { app_id } = req.params;
    const signature = req.headers['payments-signature'];
    const rawBody = req.rawBody;
    const event = req.body;

    if (!signature) {
      return res.status(401).json({ error: 'Missing webhook signature' });
    }

    if (!rawBody) {
      logger.error('Missing rawBody - captureRawBody middleware not configured');
      return res.status(500).json({ error: 'Server configuration error' });
    }

    const result = await billingService.processWebhook(app_id, event, rawBody, signature);

    if (!result.success) {
      return res.status(401).json({ error: result.error || 'Invalid webhook signature' });
    }

    res.json({ received: true, duplicate: result.duplicate || false });
  } catch (error) {
    next(error);
  }
});

// ===========================================================
// ONE-TIME CHARGES
// ===========================================================

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
router.post('/charges/one-time', requireAppId(), rejectSensitiveData, createOneTimeChargeValidator, async (req, res, next) => {
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

// ===========================================================
// REFUNDS
// ===========================================================

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
router.post('/refunds', requireAppId(), rejectSensitiveData, createRefundValidator, async (req, res, next) => {
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

// ============================================================================
// TAX ENDPOINTS (PHASE 1)
// ============================================================================

// GET /api/billing/tax-rates/:jurisdictionCode
router.get('/tax-rates/:jurisdictionCode', requireAppId(), getTaxRatesByJurisdictionValidator, async (req, res, next) => {
  try {
    const { jurisdictionCode } = req.params;
    const appId = req.verifiedAppId;

    const taxRates = await billingService.getTaxRatesByJurisdiction(appId, jurisdictionCode);
    res.json({ tax_rates: taxRates });
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/tax-rates
router.post('/tax-rates', requireAppId(), createTaxRateValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const { jurisdiction_code, tax_type, rate, effective_date, expiration_date, description, metadata } = req.body;

    const options = {};
    if (effective_date) options.effectiveDate = new Date(effective_date);
    if (expiration_date) options.expirationDate = new Date(expiration_date);
    if (description) options.description = description;
    if (metadata) options.metadata = metadata;

    const taxRate = await billingService.createTaxRate(
      appId,
      jurisdiction_code,
      tax_type,
      parseFloat(rate),
      options
    );

    res.status(201).json({ tax_rate: taxRate });
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/tax-exemptions
router.post('/tax-exemptions', requireAppId(), createTaxExemptionValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const { customer_id, tax_type, certificate_number } = req.body;

    const exemption = await billingService.createTaxExemption(
      appId,
      Number(customer_id),
      tax_type,
      certificate_number
    );

    res.status(201).json({ tax_exemption: exemption });
  } catch (error) {
    next(error);
  }
});

// GET /api/billing/tax-calculations/invoice/:invoiceId
router.get('/tax-calculations/invoice/:invoiceId', requireAppId(), getTaxCalculationsForInvoiceValidator, async (req, res, next) => {
  try {
    const { invoiceId } = req.params;
    const appId = req.verifiedAppId;

    const taxCalculations = await billingService.getTaxCalculationsForInvoice(appId, Number(invoiceId));
    res.json({ tax_calculations: taxCalculations });
  } catch (error) {
    next(error);
  }
});

// ============================================================================
// PRORATION ENDPOINTS (PHASE 3)
// ============================================================================

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
router.post('/proration/calculate', requireAppId(), rejectSensitiveData, calculateProrationValidator, async (req, res, next) => {
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
router.post('/subscriptions/:subscription_id/proration/apply', requireAppId(), rejectSensitiveData, applySubscriptionChangeValidator, async (req, res, next) => {
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

    const result = await billingService.applySubscriptionChange(
      Number(subscription_id),
      changeDetails,
      options
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
router.post('/subscriptions/:subscription_id/proration/cancellation-refund', requireAppId(), rejectSensitiveData, calculateCancellationRefundValidator, async (req, res, next) => {
  try {
    const { subscription_id } = req.params;
    const { cancellation_date, refund_behavior = 'partial_refund' } = req.body;

    const cancellationRefund = await billingService.calculateCancellationRefund(
      Number(subscription_id),
      new Date(cancellation_date),
      refund_behavior
    );

    res.json({ cancellation_refund: cancellationRefund });
  } catch (error) {
    next(error);
  }
});

/**
 * POST /usage/record
 * Record metered usage for a customer/subscription
 *
 * Body:
 *   {
 *     customer_id: integer,
 *     subscription_id: integer (optional),
 *     metric_name: string,
 *     quantity: number (decimal),
 *     unit_price_cents: integer,
 *     period_start: string (ISO 8601),
 *     period_end: string (ISO 8601),
 *     metadata: object (optional)
 *   }
 *
 * Response:
 *   {
 *     usage_record: { ... } // Created usage record from UsageService.recordUsage
 *   }
 */
router.post('/usage/record', requireAppId(), rejectSensitiveData, recordUsageValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const {
      customer_id,
      subscription_id,
      metric_name,
      quantity,
      unit_price_cents,
      period_start,
      period_end,
      metadata = {}
    } = req.body;

    const usageRecord = await billingService.usageService.recordUsage({
      appId,
      customerId: customer_id,
      subscriptionId: subscription_id,
      metricName: metric_name,
      quantity,
      unitPriceCents: unit_price_cents,
      periodStart: new Date(period_start),
      periodEnd: new Date(period_end),
      metadata
    });

    res.status(201).json({ usage_record: usageRecord });
  } catch (error) {
    next(error);
  }
});

/**
 * POST /usage/calculate-charges
 * Calculate usage charges for a billing period
 *
 * Body:
 *   {
 *     customer_id: integer,
 *     subscription_id: integer (optional),
 *     billing_period_start: string (ISO 8601),
 *     billing_period_end: string (ISO 8601),
 *     create_charges: boolean (optional, default: false)
 *   }
 *
 * Response:
 *   {
 *     usage_calculation: { ... } // Calculation result from UsageService.calculateUsageCharges
 *   }
 */
router.post('/usage/calculate-charges', requireAppId(), rejectSensitiveData, calculateUsageChargesValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const {
      customer_id,
      subscription_id,
      billing_period_start,
      billing_period_end,
      create_charges = false
    } = req.body;

    const usageCalculation = await billingService.usageService.calculateUsageCharges({
      appId,
      customerId: customer_id,
      subscriptionId: subscription_id,
      billingPeriodStart: new Date(billing_period_start),
      billingPeriodEnd: new Date(billing_period_end),
      createCharges: create_charges
    });

    res.json({ usage_calculation: usageCalculation });
  } catch (error) {
    next(error);
  }
});

/**
 * GET /usage/report
 * Get usage report for a customer/subscription
 *
 * Query parameters:
 *   customer_id: integer,
 *   subscription_id: integer (optional),
 *   start_date: string (ISO 8601),
 *   end_date: string (ISO 8601),
 *   include_billed: boolean (optional, default: true),
 *   include_unbilled: boolean (optional, default: true)
 *
 * Response:
 *   {
 *     usage_report: { ... } // Report from UsageService.getUsageReport
 *   }
 */
router.get('/usage/report', requireAppId(), rejectSensitiveData, getUsageReportValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const {
      customer_id,
      subscription_id,
      start_date,
      end_date,
      include_billed = true,
      include_unbilled = true
    } = req.query;

    const usageReport = await billingService.usageService.getUsageReport({
      appId,
      customerId: parseInt(customer_id, 10),
      subscriptionId: subscription_id ? parseInt(subscription_id, 10) : null,
      startDate: new Date(start_date),
      endDate: new Date(end_date),
      includeBilled: include_billed === 'true' || include_billed === true,
      includeUnbilled: include_unbilled === 'true' || include_unbilled === true
    });

    res.json({ usage_report: usageReport });
  } catch (error) {
    next(error);
  }
});

module.exports = router;
