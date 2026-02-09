const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { getTilledClient } = require('../tilledClientFactory');
const SubscriptionService = require('../services/SubscriptionService');
const DunningConfigService = require('../services/DunningConfigService');
const PaymentRetryService = require('../services/PaymentRetryService');

/**
 * DunningAdvancementJob - Scheduled job for advancing customers through dunning stages
 *
 * Processes delinquent customers through the dunning workflow:
 * - Grace period: send reminders
 * - Retry scheduled: wait for next retry date
 * - Retry due: attempt payment retry via PaymentRetryService
 * - Expired: mark subscriptions as past_due after max retries reached
 *
 * Uses configurable grace periods and retry schedules from DunningConfigService.
 */
class DunningAdvancementJob {
  constructor() {
    this.subscriptionService = new SubscriptionService(getTilledClient);
    this.dunningConfigService = new DunningConfigService();
    this.paymentRetryService = new PaymentRetryService();
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
   * Determine dunning stage based on grace period and retry schedule
   * @param {Object} customer - Customer record
   * @param {Object} config - Dunning configuration
   * @returns {Object} Stage info with stage, retryAttempt, nextRetryDate, etc.
   */
  async determineCustomerStage(customer, config) {
    const now = new Date();

    // If no grace period set, assume expired
    if (!customer.grace_period_end) {
      return {
        stage: 'expired',
        gracePeriodEnd: null,
        retryAttempt: customer.retry_attempt_count || 0,
        maxRetryAttempts: config.maxRetryAttempts,
        nextRetryAt: customer.next_retry_at
      };
    }

    // Check if still in grace period
    if (customer.grace_period_end > now) {
      return {
        stage: 'grace_period',
        gracePeriodEnd: customer.grace_period_end,
        retryAttempt: 0,
        maxRetryAttempts: config.maxRetryAttempts,
        nextRetryAt: null
      };
    }

    // Grace period expired - check retry status
    const retryAttempt = customer.retry_attempt_count || 0;

    // If max retry attempts reached, mark as expired
    if (retryAttempt >= config.maxRetryAttempts) {
      return {
        stage: 'expired',
        gracePeriodEnd: customer.grace_period_end,
        retryAttempt,
        maxRetryAttempts: config.maxRetryAttempts,
        nextRetryAt: null
      };
    }

    // Check if next retry is scheduled
    if (customer.next_retry_at) {
      if (customer.next_retry_at > now) {
        return {
          stage: 'retry_scheduled',
          gracePeriodEnd: customer.grace_period_end,
          retryAttempt,
          maxRetryAttempts: config.maxRetryAttempts,
          nextRetryAt: customer.next_retry_at
        };
      } else {
        return {
          stage: 'retry_due',
          gracePeriodEnd: customer.grace_period_end,
          retryAttempt,
          maxRetryAttempts: config.maxRetryAttempts,
          nextRetryAt: customer.next_retry_at
        };
      }
    }

    // No retry scheduled yet - calculate first retry based on schedule
    const daysSinceGraceEnd = Math.floor((now - customer.grace_period_end) / (1000 * 60 * 60 * 24));
    const retrySchedule = config.retryScheduleDays || [];

    // Find appropriate retry day in schedule
    let scheduledRetryDay = null;
    for (const retryDay of retrySchedule) {
      if (retryDay >= daysSinceGraceEnd) {
        scheduledRetryDay = retryDay;
        break;
      }
    }

    if (scheduledRetryDay !== null) {
      // Schedule retry for the appropriate day
      const nextRetryDate = new Date(customer.grace_period_end);
      nextRetryDate.setDate(nextRetryDate.getDate() + scheduledRetryDay);

      return {
        stage: 'retry_scheduled',
        gracePeriodEnd: customer.grace_period_end,
        retryAttempt,
        maxRetryAttempts: config.maxRetryAttempts,
        nextRetryAt: nextRetryDate,
        scheduledRetryDay
      };
    } else {
      // No more retry days in schedule - mark as expired
      return {
        stage: 'expired',
        gracePeriodEnd: customer.grace_period_end,
        retryAttempt,
        maxRetryAttempts: config.maxRetryAttempts,
        nextRetryAt: null
      };
    }
  }

  /**
   * Determine dunning stage based on grace period (legacy method)
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
   * @param {Object} config - Dunning configuration
   * @returns {Promise<Object>} Result of processing
   */
  async processDelinquentCustomer(customer, config) {
    const stageInfo = await this.determineCustomerStage(customer, config);
    const { stage } = stageInfo;

    if (stage === 'grace_period') {
      // Customer is in grace period - send reminder
      await this.sendReminder(customer, stage);
      return {
        customerId: customer.id,
        stage,
        action: 'reminder_sent',
        stageInfo
      };
    } else if (stage === 'retry_scheduled') {
      // Retry is scheduled but not due yet - do nothing
      return {
        customerId: customer.id,
        stage,
        action: 'waiting_for_retry',
        stageInfo,
        nextRetryAt: stageInfo.nextRetryAt
      };
    } else if (stage === 'retry_due') {
      // Retry is due - attempt payment retry
      const retryResult = await this.paymentRetryService.processCustomerRetry(
        customer.app_id,
        customer,
        config
      );

      return {
        customerId: customer.id,
        stage,
        action: 'retry_attempted',
        stageInfo,
        retryResult
      };
    } else if (stage === 'expired') {
      // Grace period expired and max retries reached - advance to expired stage
      const result = await this.advanceToExpiredStage(customer);
      return {
        customerId: customer.id,
        stage,
        action: 'advanced_to_expired',
        stageInfo,
        updatedSubscriptions: result.updatedSubscriptions
      };
    }

    return {
      customerId: customer.id,
      stage: 'unknown',
      action: 'none',
      stageInfo
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

    // Get dunning configuration for app
    let config;
    try {
      config = await this.dunningConfigService.getConfig(appId);
      logger.info('Loaded dunning config', {
        app_id: appId,
        grace_period_days: config.gracePeriodDays,
        max_retry_attempts: config.maxRetryAttempts,
        retry_schedule_days: config.retryScheduleDays
      });
    } catch (error) {
      logger.error('Failed to load dunning config', {
        app_id: appId,
        error: error.message
      });
      throw new Error(`Failed to load dunning configuration: ${error.message}`);
    }

    const customers = await this.findDelinquentCustomers(appId);
    logger.info(`Found ${customers.length} delinquent customers`, { app_id: appId });

    const results = {
      processed: 0,
      gracePeriod: 0,
      retryScheduled: 0,
      retryDue: 0,
      expired: 0,
      errors: 0,
      details: []
    };

    for (const customer of customers) {
      try {
        const result = await this.processDelinquentCustomer(customer, config);
        results.details.push(result);
        results.processed++;
        if (result.stage === 'grace_period') results.gracePeriod++;
        if (result.stage === 'retry_scheduled') results.retryScheduled++;
        if (result.stage === 'retry_due') results.retryDue++;
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