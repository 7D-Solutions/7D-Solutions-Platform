const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { getTilledClient } = require('../tilledClientFactory');
const InvoiceService = require('../services/InvoiceService');

/**
 * RenewalProcessingJob - Scheduled job for generating renewal invoices
 *
 * Queries active subscriptions nearing current_period_end, generates invoices
 * for the next billing period. Excludes subscriptions marked for cancellation.
 */
class RenewalProcessingJob {
  constructor() {
    this.invoiceService = new InvoiceService(getTilledClient);
    // Configurable window: subscriptions due within next 24 hours
    this.renewalWindowHours = 24;
  }

  /**
   * Calculate next period end date based on subscription interval
   * @param {Date} periodStart - Start date of next period
   * @param {string} intervalUnit - 'day', 'week', 'month', 'year'
   * @param {number} intervalCount - Number of interval units
   * @returns {Date} Period end date
   */
  calculateNextPeriodEnd(periodStart, intervalUnit, intervalCount) {
    const result = new Date(periodStart);
    switch (intervalUnit) {
      case 'day':
        result.setDate(result.getDate() + intervalCount);
        break;
      case 'week':
        result.setDate(result.getDate() + intervalCount * 7);
        break;
      case 'month':
        result.setMonth(result.getMonth() + intervalCount);
        break;
      case 'year':
        result.setFullYear(result.getFullYear() + intervalCount);
        break;
      default:
        throw new Error(`Unsupported interval unit: ${intervalUnit}`);
    }
    return result;
  }

  /**
   * Find subscriptions due for renewal within the configured window
   * @param {string} appId - Optional filter by app
   * @returns {Promise<Array>} Subscriptions due for renewal
   */
  async findRenewalSubscriptions(appId = null) {
    const now = new Date();
    const windowEnd = new Date(now.getTime() + this.renewalWindowHours * 60 * 60 * 1000);

    const where = {
      status: 'active',
      cancel_at_period_end: false,
      current_period_end: {
        gte: now,
        lte: windowEnd
      }
    };
    if (appId) {
      where.app_id = appId;
    }

    // TODO: Exclude subscriptions that already have a draft/pending invoice for next period
    // This could be done via a NOT EXISTS subquery or separate check.

    const subscriptions = await billingPrisma.billing_subscriptions.findMany({
      where,
      include: {
        billing_customers: true
      }
    });

    return subscriptions;
  }

  /**
   * Process renewal for a single subscription
   * @param {Object} subscription - Subscription record with included customer
   * @returns {Promise<Object>} Result with invoice ID or error
   */
  async processSubscriptionRenewal(subscription) {
    const { app_id: appId, id: subscriptionId, current_period_end, interval_unit, interval_count } = subscription;
    const billingPeriodStart = new Date(current_period_end);
    const billingPeriodEnd = this.calculateNextPeriodEnd(billingPeriodStart, interval_unit, interval_count);

    try {
      logger.info('Generating renewal invoice', {
        app_id: appId,
        subscription_id: subscriptionId,
        billing_period_start: billingPeriodStart,
        billing_period_end: billingPeriodEnd
      });

      const invoice = await this.invoiceService.generateInvoiceFromSubscription({
        appId,
        subscriptionId,
        billingPeriodStart,
        billingPeriodEnd,
        includeUsage: true,
        includeTax: true,
        includeDiscounts: true
      });

      logger.info('Renewal invoice generated successfully', {
        app_id: appId,
        subscription_id: subscriptionId,
        invoice_id: invoice.id
      });

      return {
        success: true,
        subscriptionId,
        invoiceId: invoice.id,
        invoiceStatus: invoice.status
      };
    } catch (error) {
      logger.error('Failed to generate renewal invoice', {
        app_id: appId,
        subscription_id: subscriptionId,
        error: error.message,
        stack: error.stack
      });
      return {
        success: false,
        subscriptionId,
        error: error.message
      };
    }
  }

  /**
   * Main job entry point: find and process renewals
   * @param {Object} options
   * @param {string} options.appId - Filter by app
   * @returns {Promise<Object>} Summary of processed renewals
   */
  async runRenewalJob(options = {}) {
    const { appId } = options;
    const startTime = Date.now();
    logger.info('Starting renewal processing job', { app_id: appId });

    const subscriptions = await this.findRenewalSubscriptions(appId);
    logger.info(`Found ${subscriptions.length} subscriptions due for renewal`, { app_id: appId });

    const results = {
      processed: 0,
      succeeded: 0,
      failed: 0,
      details: []
    };

    for (const subscription of subscriptions) {
      const result = await this.processSubscriptionRenewal(subscription);
      results.details.push(result);
      if (result.success) {
        results.succeeded++;
      } else {
        results.failed++;
      }
      results.processed++;
    }

    const duration = Date.now() - startTime;
    logger.info('Renewal processing job completed', {
      app_id: appId,
      duration: `${duration}ms`,
      ...results
    });

    return results;
  }
}

module.exports = RenewalProcessingJob;