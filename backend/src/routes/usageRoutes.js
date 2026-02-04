const express = require('express');
const BillingService = require('../billingService');
const { requireAppId, rejectSensitiveData } = require('../middleware');
const {
  recordUsageValidator,
  calculateUsageChargesValidator,
  getUsageReportValidator
} = require('../validators/usageValidators');

const router = express.Router();
const billingService = new BillingService();

// Apply requireAppId middleware to all routes in this file
router.use(requireAppId());

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
router.post('/record', rejectSensitiveData, recordUsageValidator, async (req, res, next) => {
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
router.post('/calculate-charges', rejectSensitiveData, calculateUsageChargesValidator, async (req, res, next) => {
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
router.get('/report', rejectSensitiveData, getUsageReportValidator, async (req, res, next) => {
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