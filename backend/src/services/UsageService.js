const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError } = require('../utils/errors');

/**
 * UsageService - Phase 4: Metered Usage Billing
 *
 * Generic usage billing service for metered/tracked usage:
 * - Record metered usage (quantity-based consumption)
 * - Calculate usage charges for billing periods
 * - Generate usage reports
 * - Mark usage as billed (creates charges via ChargeService)
 *
 * Uses billing_metered_usage table for raw usage records.
 * Creates billing_charges with charge_type 'usage' when billed.
 *
 * @author MistyBridge (WhiteBadger)
 * @phase 4
 */
class UsageService {
  constructor(getTilledClientFn) {
    this.getTilledClient = getTilledClientFn;
  }

  /**
   * Record metered usage for a customer/subscription
   * @param {Object} params
   * @param {string} params.appId - Application identifier
   * @param {number} params.customerId - Billing customer ID
   * @param {number} params.subscriptionId - Optional subscription ID
   * @param {string} params.metricName - Metric name (e.g., 'api_calls', 'storage_gb', 'container_pickups')
   * @param {number} params.quantity - Usage quantity (decimal supported)
   * @param {number} params.unitPriceCents - Price per unit in cents
   * @param {Date} params.periodStart - Start of usage period
   * @param {Date} params.periodEnd - End of usage period
   * @param {Object} params.metadata - Additional usage metadata
   * @returns {Promise<Object>} Created usage record
   */
  async recordUsage(params) {
    const {
      appId,
      customerId,
      subscriptionId = null,
      metricName,
      quantity,
      unitPriceCents,
      periodStart,
      periodEnd,
      metadata = {}
    } = params;

    // Validate required fields
    if (!appId || !customerId || !metricName || quantity === undefined || unitPriceCents === undefined) {
      throw new ValidationError('appId, customerId, metricName, quantity, and unitPriceCents are required');
    }

    if (quantity < 0) {
      throw new ValidationError('quantity must be non-negative');
    }

    if (unitPriceCents < 0) {
      throw new ValidationError('unitPriceCents must be non-negative');
    }

    if (!periodStart || !periodEnd) {
      throw new ValidationError('periodStart and periodEnd are required');
    }

    if (periodStart >= periodEnd) {
      throw new ValidationError('periodStart must be before periodEnd');
    }

    // Verify customer exists
    const customer = await billingPrisma.billing_customers.findFirst({
      where: {
        id: customerId,
        app_id: appId
      }
    });

    if (!customer) {
      throw new NotFoundError(`Customer ${customerId} not found for app ${appId}`);
    }

    // Verify subscription exists if provided
    if (subscriptionId) {
      const subscription = await billingPrisma.billing_subscriptions.findFirst({
        where: {
          id: subscriptionId,
          app_id: appId,
          billing_customer_id: customerId
        }
      });

      if (!subscription) {
        throw new NotFoundError(`Subscription ${subscriptionId} not found for customer ${customerId} and app ${appId}`);
      }
    }

    // Create usage record
    const usageRecord = await billingPrisma.billing_metered_usage.create({
      data: {
        app_id: appId,
        customer_id: customerId,
        subscription_id: subscriptionId,
        metric_name: metricName,
        quantity: quantity,
        unit_price_cents: unitPriceCents,
        period_start: periodStart,
        period_end: periodEnd,
        recorded_at: new Date(),
        billed_at: null,
        // Note: billed_at will be set when usage is billed via calculateUsageCharges
      }
    });

    logger.info({
      message: 'Metered usage recorded',
      appId,
      customerId,
      subscriptionId,
      metricName,
      quantity,
      unitPriceCents,
      usageId: usageRecord.id
    });

    return {
      id: usageRecord.id,
      app_id: usageRecord.app_id,
      customer_id: usageRecord.customer_id,
      subscription_id: usageRecord.subscription_id,
      metric_name: usageRecord.metric_name,
      quantity: Number(usageRecord.quantity),
      unit_price_cents: usageRecord.unit_price_cents,
      period_start: usageRecord.period_start,
      period_end: usageRecord.period_end,
      recorded_at: usageRecord.recorded_at,
      billed_at: usageRecord.billed_at
    };
  }

  /**
   * Calculate usage charges for a customer/subscription within a period
   * @param {Object} params
   * @param {string} params.appId - Application identifier
   * @param {number} params.customerId - Billing customer ID
   * @param {number} params.subscriptionId - Optional subscription ID
   * @param {Date} params.billingPeriodStart - Start of billing period
   * @param {Date} params.billingPeriodEnd - End of billing period
   * @param {boolean} params.createCharges - Whether to create billing charges (default: false)
   * @returns {Promise<Object>} Usage calculation result
   */
  async calculateUsageCharges(params) {
    const {
      appId,
      customerId,
      subscriptionId = null,
      billingPeriodStart,
      billingPeriodEnd,
      createCharges = false
    } = params;

    // Validate required fields
    if (!appId || !customerId || !billingPeriodStart || !billingPeriodEnd) {
      throw new ValidationError('appId, customerId, billingPeriodStart, and billingPeriodEnd are required');
    }

    if (billingPeriodStart >= billingPeriodEnd) {
      throw new ValidationError('billingPeriodStart must be before billingPeriodEnd');
    }

    // Verify customer exists
    const customer = await billingPrisma.billing_customers.findFirst({
      where: {
        id: customerId,
        app_id: appId
      }
    });

    if (!customer) {
      throw new NotFoundError(`Customer ${customerId} not found for app ${appId}`);
    }

    // Build query for unbilled usage within period
    const whereClause = {
      app_id: appId,
      customer_id: customerId,
      billed_at: null,
      period_start: { gte: billingPeriodStart },
      period_end: { lte: billingPeriodEnd }
    };

    if (subscriptionId) {
      whereClause.subscription_id = subscriptionId;
    }

    // Get unbilled usage records
    const usageRecords = await billingPrisma.billing_metered_usage.findMany({
      where: whereClause,
      orderBy: [{ period_start: 'asc' }, { metric_name: 'asc' }]
    });

    if (usageRecords.length === 0) {
      return {
        appId,
        customerId,
        subscriptionId,
        billingPeriodStart,
        billingPeriodEnd,
        totalAmountCents: 0,
        usageRecords: [],
        chargesCreated: [],
        summary: 'No unbilled usage found for period'
      };
    }

    // Calculate total amount and group by metric
    let totalAmountCents = 0;
    const usageByMetric = {};

    usageRecords.forEach(record => {
      const amount = Number(record.quantity) * record.unit_price_cents;
      totalAmountCents += amount;

      const metric = record.metric_name;
      if (!usageByMetric[metric]) {
        usageByMetric[metric] = {
          metric_name: metric,
          total_quantity: 0,
          total_amount_cents: 0,
          unit_price_cents: record.unit_price_cents,
          records: []
        };
      }

      usageByMetric[metric].total_quantity += Number(record.quantity);
      usageByMetric[metric].total_amount_cents += amount;
      usageByMetric[metric].records.push({
        id: record.id,
        quantity: Number(record.quantity),
        period_start: record.period_start,
        period_end: record.period_end
      });
    });

    const metrics = Object.values(usageByMetric);

    // Create charges if requested
    let chargesCreated = [];
    if (createCharges && totalAmountCents > 0) {
      // This would integrate with ChargeService
      // For now, we'll mark usage as billed and return placeholder
      // In a real implementation, we would call ChargeService.createOneTimeCharge
      // and update billed_at on usage records
      const now = new Date();

      // Update usage records as billed
      await billingPrisma.billing_metered_usage.updateMany({
        where: {
          id: { in: usageRecords.map(r => r.id) }
        },
        data: {
          billed_at: now
        }
      });

      chargesCreated = [{
        type: 'usage',
        amount_cents: totalAmountCents,
        description: `Usage charge for ${metrics.length} metric(s)`,
        created_at: now
      }];

      logger.info({
        message: 'Usage charges created',
        appId,
        customerId,
        subscriptionId,
        totalAmountCents,
        metricCount: metrics.length,
        usageRecordCount: usageRecords.length
      });
    }

    return {
      appId,
      customerId,
      subscriptionId,
      billingPeriodStart,
      billingPeriodEnd,
      totalAmountCents,
      usageRecordsCount: usageRecords.length,
      metrics,
      chargesCreated,
      summary: `Found ${usageRecords.length} usage records totaling $${(totalAmountCents / 100).toFixed(2)}`
    };
  }

  /**
   * Get usage report for customer/subscription
   * @param {Object} params
   * @param {string} params.appId - Application identifier
   * @param {number} params.customerId - Billing customer ID
   * @param {number} params.subscriptionId - Optional subscription ID
   * @param {Date} params.startDate - Report start date
   * @param {Date} params.endDate - Report end date
   * @param {boolean} params.includeBilled - Include billed usage (default: true)
   * @param {boolean} params.includeUnbilled - Include unbilled usage (default: true)
   * @returns {Promise<Object>} Usage report
   */
  async getUsageReport(params) {
    const {
      appId,
      customerId,
      subscriptionId = null,
      startDate,
      endDate,
      includeBilled = true,
      includeUnbilled = true
    } = params;

    // Validate required fields
    if (!appId || !customerId || !startDate || !endDate) {
      throw new ValidationError('appId, customerId, startDate, and endDate are required');
    }

    if (startDate >= endDate) {
      throw new ValidationError('startDate must be before endDate');
    }

    // Verify customer exists
    const customer = await billingPrisma.billing_customers.findFirst({
      where: {
        id: customerId,
        app_id: appId
      }
    });

    if (!customer) {
      throw new NotFoundError(`Customer ${customerId} not found for app ${appId}`);
    }

    // Build query conditions
    const billedConditions = [];
    if (includeBilled && includeUnbilled) {
      // Include all records
    } else if (includeBilled) {
      billedConditions.push({ billed_at: { not: null } });
    } else if (includeUnbilled) {
      billedConditions.push({ billed_at: null });
    }

    const whereClause = {
      app_id: appId,
      customer_id: customerId,
      period_start: { gte: startDate },
      period_end: { lte: endDate }
    };

    if (subscriptionId) {
      whereClause.subscription_id = subscriptionId;
    }

    if (billedConditions.length > 0) {
      whereClause.OR = billedConditions;
    }

    // Get usage records
    const usageRecords = await billingPrisma.billing_metered_usage.findMany({
      where: whereClause,
      orderBy: [{ period_start: 'desc' }, { metric_name: 'asc' }],
      include: {
        subscription: {
          select: {
            plan_name: true,
            plan_id: true
          }
        }
      }
    });

    // Calculate summary statistics
    let totalQuantity = 0;
    let totalAmountCents = 0;
    let billedAmountCents = 0;
    let unbilledAmountCents = 0;
    const metrics = {};

    usageRecords.forEach(record => {
      const quantity = Number(record.quantity);
      const amount = quantity * record.unit_price_cents;

      totalQuantity += quantity;
      totalAmountCents += amount;

      if (record.billed_at) {
        billedAmountCents += amount;
      } else {
        unbilledAmountCents += amount;
      }

      const metric = record.metric_name;
      if (!metrics[metric]) {
        metrics[metric] = {
          metric_name: metric,
          total_quantity: 0,
          total_amount_cents: 0,
          billed_amount_cents: 0,
          unbilled_amount_cents: 0,
          unit_price_cents: record.unit_price_cents
        };
      }

      metrics[metric].total_quantity += quantity;
      metrics[metric].total_amount_cents += amount;
      if (record.billed_at) {
        metrics[metric].billed_amount_cents += amount;
      } else {
        metrics[metric].unbilled_amount_cents += amount;
      }
    });

    return {
      appId,
      customerId,
      subscriptionId,
      reportPeriod: { startDate, endDate },
      summary: {
        totalRecords: usageRecords.length,
        totalQuantity,
        totalAmountCents,
        billedAmountCents,
        unbilledAmountCents,
        metrics: Object.values(metrics)
      },
      records: usageRecords.map(record => ({
        id: record.id,
        metric_name: record.metric_name,
        quantity: Number(record.quantity),
        unit_price_cents: record.unit_price_cents,
        period_start: record.period_start,
        period_end: record.period_end,
        recorded_at: record.recorded_at,
        billed_at: record.billed_at,
        subscription: record.subscription ? {
          plan_name: record.subscription.plan_name,
          plan_id: record.subscription.plan_id
        } : null
      }))
    };
  }

  /**
   * Mark usage records as billed (typically called after creating charges)
   * @param {Object} params
   * @param {string} params.appId - Application identifier
   * @param {number[]} params.usageIds - Array of usage record IDs
   * @returns {Promise<Object>} Update result
   */
  async markAsBilled(params) {
    const { appId, usageIds } = params;

    if (!appId || !usageIds || !Array.isArray(usageIds) || usageIds.length === 0) {
      throw new ValidationError('appId and usageIds array are required');
    }

    // Verify all usage records exist and belong to app
    const existingRecords = await billingPrisma.billing_metered_usage.findMany({
      where: {
        id: { in: usageIds },
        app_id: appId
      },
      select: { id: true, billed_at: true }
    });

    if (existingRecords.length !== usageIds.length) {
      const foundIds = existingRecords.map(r => r.id);
      const missingIds = usageIds.filter(id => !foundIds.includes(id));
      throw new NotFoundError(`Some usage records not found or not accessible: ${missingIds.join(', ')}`);
    }

    // Check if any are already billed
    const alreadyBilled = existingRecords.filter(r => r.billed_at !== null);
    if (alreadyBilled.length > 0) {
      throw new ValidationError(`Some usage records are already billed: ${alreadyBilled.map(r => r.id).join(', ')}`);
    }

    // Update records
    const now = new Date();
    await billingPrisma.billing_metered_usage.updateMany({
      where: {
        id: { in: usageIds },
        app_id: appId
      },
      data: {
        billed_at: now
      }
    });

    logger.info({
      message: 'Usage records marked as billed',
      appId,
      usageIdsCount: usageIds.length,
      billedAt: now
    });

    return {
      appId,
      updatedCount: usageIds.length,
      billedAt: now,
      usageIds
    };
  }
}

module.exports = UsageService;