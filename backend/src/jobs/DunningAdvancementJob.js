const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { getTilledClient } = require('../tilledClientFactory');
const SubscriptionService = require('../services/SubscriptionService');

/**
 * DunningAdvancementJob - Scheduled job for advancing customers through dunning stages
 *
 * Processes delinquent customers:
 * - Grace period not expired: send reminders (placeholder)
 * - Grace period expired: mark subscriptions as past_due or cancel
 */
class DunningAdvancementJob {
  constructor() {
    this.subscriptionService = new SubscriptionService(getTilledClient);
  }

  /**
   * Find delinquent customers (with delinquent_since not null)
   * @param {string} appId - Optional filter by app
   * @returns {Promise<Array>} Delinquent customers with their subscriptions
   */
  async findDelinquentCustomers(appId = null) {
    const where = {
      delinquent_since: { not: null }
    };
    if (appId) {
      where.app_id = appId;
    }

    const customers = await billingPrisma.billing_customers.findMany({
      where,
      include: {
        billing_subscriptions: {
          where: { status: 'active' } // Only active subscriptions
        }
      }
    });

    return customers;
  }

  /**
   * Determine dunning stage based on grace period
   * @param {Object} customer - Customer record
   * @returns {string} Stage: 'grace_period', 'expired', 'unknown'
   */
  getDunningStage(customer) {
    const now = new Date();
    if (!customer.grace_period_end) {
      // No grace period set - assume grace period expired
      return 'expired';
    }
    if (customer.grace_period_end > now) {
      return 'grace_period';
    }
    return 'expired';
  }

  /**
   * Send reminder (placeholder - integrate with notification system)
   * @param {Object} customer - Customer record
   * @param {string} stage - Current stage
   */
  async sendReminder(customer, stage) {
    logger.info('Dunning reminder placeholder', {
      app_id: customer.app_id,
      customer_id: customer.id,
      stage,
      grace_period_end: customer.grace_period_end
    });
    // TODO: Integrate with notification service (email, in-app, etc.)
  }

  /**
   * Advance customer to expired stage: mark subscriptions as past_due
   * @param {Object} customer - Customer record
   * @returns {Promise<Object>} Result with updated subscriptions count
   */
  async advanceToExpiredStage(customer) {
    const { billing_subscriptions: subscriptions } = customer;
    const updatedSubscriptions = [];

    for (const subscription of subscriptions) {
      try {
        // Update subscription status to past_due
        await billingPrisma.billing_subscriptions.update({
          where: { id: subscription.id },
          data: { status: 'past_due' }
        });
        updatedSubscriptions.push(subscription.id);
        logger.info('Subscription marked as past_due due to expired grace period', {
          app_id: customer.app_id,
          customer_id: customer.id,
          subscription_id: subscription.id
        });
      } catch (error) {
        logger.error('Failed to update subscription status', {
          app_id: customer.app_id,
          customer_id: customer.id,
          subscription_id: subscription.id,
          error: error.message
        });
      }
    }

    // Optionally update customer status to 'delinquent' or similar
    await billingPrisma.billing_customers.update({
      where: { id: customer.id },
      data: { status: 'delinquent' }
    });

    return { updatedSubscriptions };
  }

  /**
   * Process a single delinquent customer
   * @param {Object} customer - Customer record with subscriptions
   * @returns {Promise<Object>} Result of processing
   */
  async processDelinquentCustomer(customer) {
    const stage = this.getDunningStage(customer);

    if (stage === 'grace_period') {
      await this.sendReminder(customer, stage);
      return {
        customerId: customer.id,
        stage,
        action: 'reminder_sent'
      };
    } else if (stage === 'expired') {
      const result = await this.advanceToExpiredStage(customer);
      return {
        customerId: customer.id,
        stage,
        action: 'advanced_to_expired',
        updatedSubscriptions: result.updatedSubscriptions
      };
    }

    return {
      customerId: customer.id,
      stage: 'unknown',
      action: 'none'
    };
  }

  /**
   * Main job entry point: process delinquent customers
   * @param {Object} options
   * @param {string} options.appId - Filter by app
   * @returns {Promise<Object>} Summary of processed customers
   */
  async runDunningJob(options = {}) {
    const { appId } = options;
    const startTime = Date.now();
    logger.info('Starting dunning advancement job', { app_id: appId });

    const customers = await this.findDelinquentCustomers(appId);
    logger.info(`Found ${customers.length} delinquent customers`, { app_id: appId });

    const results = {
      processed: 0,
      gracePeriod: 0,
      expired: 0,
      errors: 0,
      details: []
    };

    for (const customer of customers) {
      try {
        const result = await this.processDelinquentCustomer(customer);
        results.details.push(result);
        results.processed++;
        if (result.stage === 'grace_period') results.gracePeriod++;
        if (result.stage === 'expired') results.expired++;
      } catch (error) {
        logger.error('Error processing delinquent customer', {
          app_id: customer.app_id,
          customer_id: customer.id,
          error: error.message
        });
        results.errors++;
        results.details.push({
          customerId: customer.id,
          error: error.message
        });
      }
    }

    const duration = Date.now() - startTime;
    logger.info('Dunning advancement job completed', {
      app_id: appId,
      duration: `${duration}ms`,
      ...results
    });

    return results;
  }
}

module.exports = DunningAdvancementJob;