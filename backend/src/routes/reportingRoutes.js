const express = require('express');
const { getTilledClient } = require('../tilledClientFactory');
const ReportingService = require('../services/ReportingService');
const { requireAppId, rejectSensitiveData } = require('../middleware');
const {
  getRevenueReportValidator,
  getMRRReportValidator,
  getChurnReportValidator,
  getAgingReceivablesReportValidator
} = require('../validators/reportingValidators');

const router = express.Router();
const reportingService = new ReportingService(getTilledClient);

// Apply requireAppId middleware to all routes in this file
router.use(requireAppId());

/**
 * GET /reports/revenue
 * Get revenue report for a date range
 *
 * Query parameters:
 *   start_date: string (ISO 8601, required)
 *   end_date: string (ISO 8601, required)
 *   granularity: string (optional, 'daily', 'weekly', 'monthly', 'quarterly', default: 'daily')
 *   customer_id: integer (optional)
 *   subscription_id: integer (optional)
 *   charge_type: string (optional, 'subscription', 'one_time', 'usage')
 *   limit: integer (optional, default: 100, max: 1000)
 *   offset: integer (optional, default: 0)
 *
 * Response:
 *   {
 *     revenue_report: {
 *       summary: {
 *         total_gross_revenue_cents: integer,
 *         total_net_revenue_cents: integer,
 *         total_refunds_cents: integer,
 *         total_transaction_count: integer,
 *         total_refund_count: integer,
 *         period_count: integer,
 *         granularity: string
 *       },
 *       periods: [
 *         {
 *           period: string,
 *           gross_revenue_cents: integer,
 *           net_revenue_cents: integer,
 *           refunds_cents: integer,
 *           transaction_count: integer,
 *           refund_count: integer
 *         }
 *       ]
 *     }
 *   }
 */
router.get('/revenue', rejectSensitiveData, getRevenueReportValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const {
      start_date,
      end_date,
      granularity = 'daily',
      customer_id,
      subscription_id,
      charge_type,
      limit = 100,
      offset = 0
    } = req.query;

    const filters = {
      startDate: new Date(start_date),
      endDate: new Date(end_date),
      granularity,
      customerId: customer_id ? parseInt(customer_id, 10) : undefined,
      subscriptionId: subscription_id ? parseInt(subscription_id, 10) : undefined,
      chargeType: charge_type
    };

    const revenueReport = await reportingService.getRevenueReport(appId, filters);

    // Apply pagination to periods
    const paginatedPeriods = revenueReport.periods.slice(offset, offset + limit);

    res.json({
      revenue_report: {
        summary: {
          ...revenueReport.summary,
          limit: parseInt(limit, 10),
          offset: parseInt(offset, 10),
          total_periods: revenueReport.periods.length
        },
        periods: paginatedPeriods
      }
    });
  } catch (error) {
    next(error);
  }
});

/**
 * GET /reports/mrr
 * Get MRR (Monthly Recurring Revenue) report
 *
 * Query parameters:
 *   as_of_date: string (ISO 8601, required)
 *   plan_id: string (optional)
 *   include_breakdown: boolean (optional, default: true)
 *
 * Response:
 *   {
 *     mrr_report: {
 *       as_of_date: string (ISO 8601),
 *       total_mrr_cents: integer,
 *       subscription_count: integer,
 *       breakdown: [
 *         {
 *           plan_id: string,
 *           mrr_cents: integer,
 *           subscription_count: integer
 *         }
 *       ] (only if include_breakdown=true)
 *     }
 *   }
 */
router.get('/mrr', rejectSensitiveData, getMRRReportValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const {
      as_of_date,
      plan_id,
      include_breakdown = true
    } = req.query;

    const filters = {
      asOfDate: new Date(as_of_date),
      planId: plan_id,
      includeBreakdown: include_breakdown === 'true' || include_breakdown === true
    };

    const mrrReport = await reportingService.getMRRReport(appId, filters);

    res.json({ mrr_report: mrrReport });
  } catch (error) {
    next(error);
  }
});

/**
 * GET /reports/churn
 * Get churn report
 *
 * Query parameters:
 *   start_date: string (ISO 8601, required)
 *   end_date: string (ISO 8601, required)
 *   cohort_period: string (optional, 'daily', 'weekly', 'monthly', 'quarterly', default: 'monthly')
 *   plan_id: string (optional)
 *
 * Response:
 *   {
 *     churn_report: {
 *       period: {
 *         start_date: string (ISO 8601),
 *         end_date: string (ISO 8601),
 *         cohort_period: string
 *       },
 *       overall: {
 *         starting_customer_count: integer,
 *         churned_customer_count: integer,
 *         churned_revenue_cents: integer,
 *         customer_churn_rate: number
 *       },
 *       cohorts: [
 *         {
 *           cohort: string,
 *           starting_customer_count: integer,
 *           churned_customer_count: integer,
 *           churned_revenue_cents: integer,
 *           customer_churn_rate: number
 *         }
 *       ]
 *     }
 *   }
 */
router.get('/churn', rejectSensitiveData, getChurnReportValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const {
      start_date,
      end_date,
      cohort_period = 'monthly',
      plan_id
    } = req.query;

    const filters = {
      startDate: new Date(start_date),
      endDate: new Date(end_date),
      cohortPeriod: cohort_period,
      planId: plan_id
    };

    const churnReport = await reportingService.getChurnReport(appId, filters);

    res.json({ churn_report: churnReport });
  } catch (error) {
    next(error);
  }
});

/**
 * GET /reports/aging-receivables
 * Get aging receivables report
 *
 * Query parameters:
 *   as_of_date: string (ISO 8601, required)
 *   customer_id: integer (optional)
 *
 * Response:
 *   {
 *     aging_receivables_report: {
 *       as_of_date: string (ISO 8601),
 *       total_outstanding_cents: integer,
 *       total_invoice_count: integer,
 *       aging_buckets: [
 *         {
 *           bucket: string ('current', '1-30', '31-60', '61-90', '90+'),
 *           amount_cents: integer,
 *           invoice_count: integer
 *         }
 *       ]
 *     }
 *   }
 */
router.get('/aging-receivables', rejectSensitiveData, getAgingReceivablesReportValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const {
      as_of_date,
      customer_id
    } = req.query;

    const filters = {
      asOfDate: new Date(as_of_date),
      customerId: customer_id ? parseInt(customer_id, 10) : undefined
    };

    const agingReport = await reportingService.getAgingReceivablesReport(appId, filters);

    res.json({ aging_receivables_report: agingReport });
  } catch (error) {
    next(error);
  }
});

module.exports = router;