const { billingPrisma } = require('../prisma');
const { Prisma } = require('@prisma/client');
const logger = require('@fireproof/infrastructure/utils/logger');
const { ValidationError } = require('../utils/errors');

class ReportingService {
  constructor(getTilledClientFn) {
    this.getTilledClient = getTilledClientFn;
  }

  /**
   * Get revenue report for a date range
   * @param {string} appId - Application ID
   * @param {Object} filters - Report filters
   * @param {Date} filters.startDate - Start date (inclusive)
   * @param {Date} filters.endDate - End date (exclusive)
   * @param {string} [filters.granularity='daily'] - Granularity: 'daily', 'weekly', 'monthly', 'quarterly'
   * @param {number} [filters.customerId] - Filter by customer ID
   * @param {number} [filters.subscriptionId] - Filter by subscription ID
   * @param {string} [filters.chargeType] - Filter by charge type: 'subscription', 'one_time', 'usage'
   * @returns {Promise<Object>} Revenue report with summary and periods
   */
  async getRevenueReport(appId, filters) {
    const {
      startDate,
      endDate,
      granularity = 'daily',
      customerId,
      subscriptionId,
      chargeType
    } = filters;

    // Validate date range
    if (!startDate || !endDate) {
      throw new ValidationError('startDate and endDate are required');
    }
    if (endDate <= startDate) {
      throw new ValidationError('endDate must be after startDate');
    }

    // Build where clause
    const where = {
      app_id: appId,
      status: 'succeeded',
      created_at: {
        gte: startDate,
        lt: endDate
      }
    };

    if (customerId) {
      where.billing_customer_id = customerId;
    }

    if (subscriptionId) {
      where.billing_subscription_id = subscriptionId;
    }

    if (chargeType) {
      where.charge_type = chargeType;
    }

    // Determine date truncation function based on granularity
    let dateTruncFunc;
    switch (granularity) {
      case 'daily':
        dateTruncFunc = 'DATE';
        break;
      case 'weekly':
        dateTruncFunc = 'YEARWEEK'; // Returns YYYYWW format
        break;
      case 'monthly':
        dateTruncFunc = 'DATE_FORMAT';
        break;
      case 'quarterly':
        dateTruncFunc = 'QUARTER';
        break;
      default:
        throw new ValidationError(`Invalid granularity: ${granularity}. Must be one of: daily, weekly, monthly, quarterly`);
    }

    // Build group by clause
    let groupByClause;
    if (granularity === 'monthly') {
      groupByClause = `DATE_FORMAT(created_at, '%Y-%m')`;
    } else if (granularity === 'weekly') {
      groupByClause = `YEARWEEK(created_at, 3)`; // Mode 3: Monday as first day of week, week 1 is first week with 4+ days
    } else {
      groupByClause = `${dateTruncFunc}(created_at)`;
    }

    // Get gross revenue (charges)
    // Note: We need to build SQL with string interpolation for GROUP BY clause
    // since MySQL doesn't allow parameters in GROUP BY
    const chargesQuery = Prisma.sql`
      SELECT
        ${Prisma.raw(groupByClause)} as period,
        SUM(amount_cents) as gross_revenue_cents,
        COUNT(*) as transaction_count
      FROM billing_charges
      WHERE app_id = ${appId}
        AND status = 'succeeded'
        AND created_at >= ${startDate}
        AND created_at < ${endDate}
        ${customerId ? Prisma.sql`AND billing_customer_id = ${customerId}` : Prisma.sql``}
        ${subscriptionId ? Prisma.sql`AND billing_subscription_id = ${subscriptionId}` : Prisma.sql``}
        ${chargeType ? Prisma.sql`AND charge_type = ${chargeType}` : Prisma.sql``}
      GROUP BY ${Prisma.raw(groupByClause)}
      ORDER BY period
    `;
    const chargesResult = await billingPrisma.$queryRaw(chargesQuery);

    // Get refunds in same period
    const refundsQuery = Prisma.sql`
      SELECT
        ${Prisma.raw(groupByClause)} as period,
        SUM(amount_cents) as refunds_cents,
        COUNT(*) as refund_count
      FROM billing_refunds
      WHERE app_id = ${appId}
        AND status = 'succeeded'
        AND created_at >= ${startDate}
        AND created_at < ${endDate}
        ${customerId ? Prisma.sql`AND billing_customer_id = ${customerId}` : Prisma.sql``}
        ${subscriptionId ? Prisma.sql`AND billing_subscription_id = ${subscriptionId}` : Prisma.sql``}
      GROUP BY ${Prisma.raw(groupByClause)}
      ORDER BY period
    `;
    const refundsResult = await billingPrisma.$queryRaw(refundsQuery);

    // Combine results
    const chargesByPeriod = new Map();
    chargesResult.forEach(row => {
      chargesByPeriod.set(row.period.toString(), {
        period: row.period,
        gross_revenue_cents: Number(row.gross_revenue_cents) || 0,
        transaction_count: Number(row.transaction_count) || 0,
        refunds_cents: 0,
        refund_count: 0
      });
    });

    refundsResult.forEach(row => {
      const periodKey = row.period.toString();
      if (chargesByPeriod.has(periodKey)) {
        const periodData = chargesByPeriod.get(periodKey);
        periodData.refunds_cents = Number(row.refunds_cents) || 0;
        periodData.refund_count = Number(row.refund_count) || 0;
      } else {
        // Refunds without corresponding charges (should be rare)
        chargesByPeriod.set(periodKey, {
          period: row.period,
          gross_revenue_cents: 0,
          transaction_count: 0,
          refunds_cents: Number(row.refunds_cents) || 0,
          refund_count: Number(row.refund_count) || 0
        });
      }
    });

    const periods = Array.from(chargesByPeriod.values())
      .map(period => ({
        ...period,
        net_revenue_cents: period.gross_revenue_cents - period.refunds_cents
      }))
      .sort((a, b) => a.period.toString().localeCompare(b.period.toString()));

    // Calculate summary
    const summary = {
      total_gross_revenue_cents: periods.reduce((sum, p) => sum + p.gross_revenue_cents, 0),
      total_net_revenue_cents: periods.reduce((sum, p) => sum + p.net_revenue_cents, 0),
      total_refunds_cents: periods.reduce((sum, p) => sum + p.refunds_cents, 0),
      total_transaction_count: periods.reduce((sum, p) => sum + p.transaction_count, 0),
      total_refund_count: periods.reduce((sum, p) => sum + p.refund_count, 0),
      period_count: periods.length,
      granularity
    };

    return {
      summary,
      periods
    };
  }

  /**
   * Get MRR (Monthly Recurring Revenue) report
   * @param {string} appId - Application ID
   * @param {Object} filters - Report filters
   * @param {Date} filters.asOfDate - Snapshot date for MRR calculation
   * @param {string} [filters.planId] - Filter by plan ID
   * @param {boolean} [filters.includeBreakdown=true] - Include plan-level breakdown
   * @returns {Promise<Object>} MRR report
   */
  async getMRRReport(appId, filters) {
    const {
      asOfDate,
      planId,
      includeBreakdown = true
    } = filters;

    if (!asOfDate) {
      throw new ValidationError('asOfDate is required');
    }

    // Build where clause for active subscriptions
    const where = {
      app_id: appId,
      status: 'active',
      current_period_start: { lte: asOfDate },
      current_period_end: { gte: asOfDate }
    };

    if (planId) {
      where.plan_id = planId;
    }

    // Get active subscriptions
    const activeSubscriptions = await billingPrisma.billing_subscriptions.findMany({
      where,
      select: {
        id: true,
        plan_id: true,
        price_cents: true,
        interval_unit: true,
        interval_count: true
      }
    });

    // Calculate MRR for each subscription
    const subscriptionsWithMRR = activeSubscriptions.map(sub => {
      let monthlyAmount = sub.price_cents;

      // Normalize to monthly
      if (sub.interval_unit === 'year') {
        monthlyAmount = Math.round(sub.price_cents / 12);
      } else if (sub.interval_unit === 'quarter') {
        monthlyAmount = Math.round(sub.price_cents / 3);
      } else if (sub.interval_unit === 'week') {
        monthlyAmount = Math.round(sub.price_cents * 4.33); // Average weeks per month
      } else if (sub.interval_unit === 'day') {
        monthlyAmount = Math.round(sub.price_cents * 30.44); // Average days per month
      }
      // 'month' interval is already monthly

      return {
        ...sub,
        mrr_cents: monthlyAmount
      };
    });

    // Calculate total MRR
    const totalMRRCents = subscriptionsWithMRR.reduce((sum, sub) => sum + sub.mrr_cents, 0);

    const result = {
      as_of_date: asOfDate,
      total_mrr_cents: totalMRRCents,
      subscription_count: subscriptionsWithMRR.length
    };

    // Add breakdown if requested
    if (includeBreakdown) {
      const breakdownByPlan = new Map();

      subscriptionsWithMRR.forEach(sub => {
        const planKey = sub.plan_id;
        if (!breakdownByPlan.has(planKey)) {
          breakdownByPlan.set(planKey, {
            plan_id: sub.plan_id,
            mrr_cents: 0,
            subscription_count: 0
          });
        }

        const planData = breakdownByPlan.get(planKey);
        planData.mrr_cents += sub.mrr_cents;
        planData.subscription_count += 1;
      });

      result.breakdown = Array.from(breakdownByPlan.values())
        .sort((a, b) => b.mrr_cents - a.mrr_cents);
    }

    return result;
  }

  /**
   * Get churn report
   * @param {string} appId - Application ID
   * @param {Object} filters - Report filters
   * @param {Date} filters.startDate - Start date (inclusive)
   * @param {Date} filters.endDate - End date (exclusive)
   * @param {string} [filters.cohortPeriod='monthly'] - Cohort period: 'daily', 'weekly', 'monthly', 'quarterly'
   * @param {string} [filters.planId] - Filter by plan ID
   * @returns {Promise<Object>} Churn report
   */
  async getChurnReport(appId, filters) {
    const {
      startDate,
      endDate,
      cohortPeriod = 'monthly',
      planId
    } = filters;

    if (!startDate || !endDate) {
      throw new ValidationError('startDate and endDate are required');
    }
    if (endDate <= startDate) {
      throw new ValidationError('endDate must be after startDate');
    }

    // Determine date truncation for cohort grouping
    let cohortGroupFunc;
    switch (cohortPeriod) {
      case 'daily':
        cohortGroupFunc = 'DATE';
        break;
      case 'weekly':
        cohortGroupFunc = 'YEARWEEK';
        break;
      case 'monthly':
        cohortGroupFunc = 'DATE_FORMAT';
        break;
      case 'quarterly':
        cohortGroupFunc = 'QUARTER';
        break;
      default:
        throw new ValidationError(`Invalid cohortPeriod: ${cohortPeriod}. Must be one of: daily, weekly, monthly, quarterly`);
    }

    let cohortGroupByClause;
    if (cohortPeriod === 'monthly') {
      cohortGroupByClause = `DATE_FORMAT(created_at, '%Y-%m')`;
    } else if (cohortPeriod === 'weekly') {
      cohortGroupByClause = `YEARWEEK(created_at, 3)`;
    } else {
      cohortGroupByClause = `${cohortGroupFunc}(created_at)`;
    }

    // Get starting active customers for each cohort
    const startingActiveQuery = await billingPrisma.$queryRaw(
      Prisma.sql`
        SELECT
          ${Prisma.raw(cohortGroupByClause)} as cohort,
          COUNT(DISTINCT billing_customer_id) as starting_customer_count
        FROM billing_subscriptions
        WHERE app_id = ${appId}
          AND status = 'active'
          AND created_at < ${startDate}
          AND (canceled_at IS NULL OR canceled_at >= ${startDate})
          ${planId ? Prisma.sql`AND plan_id = ${planId}` : Prisma.sql``}
        GROUP BY ${Prisma.raw(cohortGroupByClause)}
      `
    );

    // Get churned customers in the period
    const churnedQuery = await billingPrisma.$queryRaw(
      Prisma.sql`
        SELECT
          ${Prisma.raw(cohortGroupByClause)} as cohort,
          COUNT(DISTINCT billing_customer_id) as churned_customer_count,
          SUM(price_cents) as churned_revenue_cents
        FROM billing_subscriptions
        WHERE app_id = ${appId}
          AND status = 'canceled'
          AND canceled_at >= ${startDate}
          AND canceled_at < ${endDate}
          ${planId ? Prisma.sql`AND plan_id = ${planId}` : Prisma.sql``}
        GROUP BY ${Prisma.raw(cohortGroupByClause)}
      `
    );

    // Combine results by cohort
    const startingByCohort = new Map();
    startingActiveQuery.forEach(row => {
      startingByCohort.set(row.cohort.toString(), {
        starting_customer_count: Number(row.starting_customer_count) || 0,
        churned_customer_count: 0,
        churned_revenue_cents: 0
      });
    });

    churnedQuery.forEach(row => {
      const cohortKey = row.cohort.toString();
      if (startingByCohort.has(cohortKey)) {
        const cohortData = startingByCohort.get(cohortKey);
        cohortData.churned_customer_count = Number(row.churned_customer_count) || 0;
        cohortData.churned_revenue_cents = Number(row.churned_revenue_cents) || 0;
      } else {
        // Churned customers from cohorts with no starting active (edge case)
        startingByCohort.set(cohortKey, {
          starting_customer_count: 0,
          churned_customer_count: Number(row.churned_customer_count) || 0,
          churned_revenue_cents: Number(row.churned_revenue_cents) || 0
        });
      }
    });

    // Calculate churn rates for each cohort
    const cohorts = Array.from(startingByCohort.entries())
      .map(([cohort, data]) => {
        const customerChurnRate = data.starting_customer_count > 0
          ? data.churned_customer_count / data.starting_customer_count
          : 0;

        return {
          cohort,
          starting_customer_count: data.starting_customer_count,
          churned_customer_count: data.churned_customer_count,
          churned_revenue_cents: data.churned_revenue_cents,
          customer_churn_rate: customerChurnRate
        };
      })
      .sort((a, b) => a.cohort.localeCompare(b.cohort));

    // Calculate overall churn rates
    const totalStartingCustomers = cohorts.reduce((sum, c) => sum + c.starting_customer_count, 0);
    const totalChurnedCustomers = cohorts.reduce((sum, c) => sum + c.churned_customer_count, 0);
    const totalChurnedRevenue = cohorts.reduce((sum, c) => sum + c.churned_revenue_cents, 0);

    const overallCustomerChurnRate = totalStartingCustomers > 0
      ? totalChurnedCustomers / totalStartingCustomers
      : 0;

    return {
      period: {
        start_date: startDate,
        end_date: endDate,
        cohort_period: cohortPeriod
      },
      overall: {
        starting_customer_count: totalStartingCustomers,
        churned_customer_count: totalChurnedCustomers,
        churned_revenue_cents: totalChurnedRevenue,
        customer_churn_rate: overallCustomerChurnRate
      },
      cohorts
    };
  }

  /**
   * Get aging receivables report
   * @param {string} appId - Application ID
   * @param {Object} filters - Report filters
   * @param {Date} filters.asOfDate - As-of date for aging calculation
   * @param {number} [filters.customerId] - Filter by customer ID
   * @returns {Promise<Object>} Aging receivables report
   */
  async getAgingReceivablesReport(appId, filters) {
    const {
      asOfDate,
      customerId
    } = filters;

    if (!asOfDate) {
      throw new ValidationError('asOfDate is required');
    }

    // Build where clause for open/past_due invoices with due dates
    const where = {
      app_id: appId,
      status: { in: ['open', 'past_due'] },
      due_at: { not: null }
    };

    if (customerId) {
      where.billing_customer_id = customerId;
    }

    // Get all outstanding invoices
    const invoices = await billingPrisma.billing_invoices.findMany({
      where,
      select: {
        id: true,
        billing_customer_id: true,
        amount_cents: true,
        due_at: true,
        status: true
      }
    });

    // Calculate aging buckets
    const agingBuckets = {
      current: { amount_cents: 0, invoice_count: 0 },
      '1-30': { amount_cents: 0, invoice_count: 0 },
      '31-60': { amount_cents: 0, invoice_count: 0 },
      '61-90': { amount_cents: 0, invoice_count: 0 },
      '90+': { amount_cents: 0, invoice_count: 0 }
    };

    invoices.forEach(invoice => {
      const daysOverdue = Math.floor((asOfDate - invoice.due_at) / (1000 * 60 * 60 * 24));

      // Skip paid, void, or uncollectible invoices
      if (invoice.status === 'paid' || invoice.status === 'void' || invoice.status === 'uncollectible') {
        return;
      }

      // For open/past_due invoices, use full amount
      const outstandingAmount = invoice.amount_cents;

      let bucket;
      if (daysOverdue <= 0) {
        bucket = 'current';
      } else if (daysOverdue <= 30) {
        bucket = '1-30';
      } else if (daysOverdue <= 60) {
        bucket = '31-60';
      } else if (daysOverdue <= 90) {
        bucket = '61-90';
      } else {
        bucket = '90+';
      }

      agingBuckets[bucket].amount_cents += outstandingAmount;
      agingBuckets[bucket].invoice_count += 1;
    });

    // Convert to array format
    const bucketsArray = Object.entries(agingBuckets)
      .filter(([_, data]) => data.amount_cents > 0)
      .map(([bucket, data]) => ({
        bucket,
        amount_cents: data.amount_cents,
        invoice_count: data.invoice_count
      }))
      .sort((a, b) => {
        // Sort by aging order: current, 1-30, 31-60, 61-90, 90+
        const order = { current: 0, '1-30': 1, '31-60': 2, '61-90': 3, '90+': 4 };
        return order[a.bucket] - order[b.bucket];
      });

    // Calculate total outstanding
    const totalOutstandingCents = bucketsArray.reduce((sum, b) => sum + b.amount_cents, 0);

    return {
      as_of_date: asOfDate,
      total_outstanding_cents: totalOutstandingCents,
      total_invoice_count: invoices.length,
      aging_buckets: bucketsArray
    };
  }
}

module.exports = ReportingService;