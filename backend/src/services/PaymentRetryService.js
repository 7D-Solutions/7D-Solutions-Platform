const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { getTilledClient } = require('../tilledClientFactory');

/**
 * PaymentRetryService - Handles payment retry attempts for delinquent customers
 */
class PaymentRetryService {
  constructor() {
    // Initialize with Tilled client factory
  }

  /**
   * Get subscription's payment method for retry
   * @param {Object} subscription - Subscription record
   * @returns {Promise<Object|null>} Payment method info or null
   */
  async getSubscriptionPaymentMethod(subscription) {
    try {
      const paymentMethod = await billingPrisma.billing_payment_methods.findFirst({
        where: {
          billing_customer_id: subscription.billing_customer_id,
          tilled_payment_method_id: subscription.payment_method_id,
          status: 'active',
          deleted_at: null
        }
      });
      return paymentMethod;
    } catch (error) {
      logger.error('Failed to get subscription payment method', {
        subscription_id: subscription.id,
        error: error.message
      });
      return null;
    }
  }

  /**
   * Attempt to retry payment for a customer's subscription
   * @param {string} appId - Application identifier
   * @param {Object} customer - Customer record
   * @param {Object} subscription - Subscription record
   * @returns {Promise<Object>} Result of retry attempt
   */
  async retrySubscriptionPayment(appId, customer, subscription) {
    const startTime = Date.now();
    const retryAttempt = (customer.retry_attempt_count || 0) + 1;

    logger.info('Attempting payment retry', {
      app_id: appId,
      customer_id: customer.id,
      subscription_id: subscription.id,
      retry_attempt: retryAttempt,
      tilled_subscription_id: subscription.tilled_subscription_id
    });

    try {
      // Get payment method for subscription
      const paymentMethod = await this.getSubscriptionPaymentMethod(subscription);
      if (!paymentMethod) {
        throw new Error(`No active payment method found for subscription ${subscription.id}`);
      }

      // TODO: Implement actual Tilled API call to retry payment
      // This is a placeholder - actual implementation should:
      // 1. Get the failed invoice/payment intent from Tilled
      // 2. Retry the payment with the same payment method
      // 3. Handle different retry scenarios (card update needed, etc.)

      // Simulate API call delay
      await new Promise(resolve => setTimeout(resolve, 100));

      // Simulate success (80% success rate for demo)
      const success = Math.random() > 0.2;

      if (success) {
        logger.info('Payment retry successful (simulated)', {
          app_id: appId,
          customer_id: customer.id,
          subscription_id: subscription.id,
          retry_attempt: retryAttempt,
          duration: Date.now() - startTime
        });

        return {
          success: true,
          retryAttempt,
          message: 'Payment retry successful (simulated)',
          subscriptionId: subscription.id,
          customerId: customer.id,
          timestamp: new Date()
        };
      } else {
        logger.warn('Payment retry failed (simulated)', {
          app_id: appId,
          customer_id: customer.id,
          subscription_id: subscription.id,
          retry_attempt: retryAttempt,
          duration: Date.now() - startTime
        });

        return {
          success: false,
          retryAttempt,
          message: 'Payment retry failed (simulated)',
          subscriptionId: subscription.id,
          customerId: customer.id,
          timestamp: new Date()
        };
      }
    } catch (error) {
      logger.error('Payment retry error', {
        app_id: appId,
        customer_id: customer.id,
        subscription_id: subscription.id,
        retry_attempt: retryAttempt,
        error: error.message,
        duration: Date.now() - startTime
      });

      return {
        success: false,
        retryAttempt,
        error: error.message,
        message: `Payment retry error: ${error.message}`,
        subscriptionId: subscription.id,
        customerId: customer.id,
        timestamp: new Date()
      };
    }
  }

  /**
   * Update customer retry status after retry attempt
   * @param {string} appId - Application identifier
   * @param {Object} customer - Customer record
   * @param {Object} retryResult - Result from retrySubscriptionPayment
   * @param {Object} config - Dunning configuration
   * @returns {Promise<Object>} Updated customer record
   */
  async updateCustomerRetryStatus(appId, customer, retryResult, config) {
    const now = new Date();
    const retryAttempt = retryResult.retryAttempt;
    const retrySchedule = config.retryScheduleDays || [];

    let nextRetryAt = null;
    let customerStatus = customer.status;

    if (retryResult.success) {
      // Payment successful - reset delinquent status
      customerStatus = 'active';
      logger.info('Payment retry successful, resetting customer status', {
        app_id: appId,
        customer_id: customer.id,
        retry_attempt: retryAttempt
      });
    } else if (retryAttempt < config.maxRetryAttempts && retrySchedule.length > retryAttempt) {
      // Schedule next retry based on schedule
      // retryAttempt is the number of attempts completed, so use it as index for next retry
      // e.g., after 1st retry (retryAttempt=1), schedule 2nd retry using retrySchedule[1]
      const scheduleIndex = retryAttempt;
      const retryDay = retrySchedule[scheduleIndex];

      // Calculate from grace_period_end, not from now
      // Retry schedule days are absolute days from grace period end
      if (customer.grace_period_end) {
        nextRetryAt = new Date(customer.grace_period_end);
        nextRetryAt.setDate(nextRetryAt.getDate() + retryDay);

        logger.info('Scheduling next retry attempt', {
          app_id: appId,
          customer_id: customer.id,
          retry_attempt: retryAttempt,
          schedule_index: scheduleIndex,
          retry_day: retryDay,
          next_retry_at: nextRetryAt
        });
      } else {
        logger.warn('Cannot schedule next retry: no grace_period_end', {
          app_id: appId,
          customer_id: customer.id,
          retry_attempt: retryAttempt
        });
      }
    } else {
      // Max retry attempts reached
      logger.info('Max retry attempts reached', {
        app_id: appId,
        customer_id: customer.id,
        retry_attempt: retryAttempt,
        max_retry_attempts: config.maxRetryAttempts
      });
    }

    // Update customer record
    const updateData = {
      retry_attempt_count: retryAttempt,
      next_retry_at: nextRetryAt,
      updated_at: now
    };

    if (customerStatus !== customer.status) {
      updateData.status = customerStatus;
    }

    if (retryResult.success) {
      // Reset delinquent status on successful payment
      updateData.delinquent_since = null;
      updateData.grace_period_end = null;
    }

    try {
      const updatedCustomer = await billingPrisma.billing_customers.update({
        where: { id: customer.id },
        data: updateData
      });

      logger.info('Customer retry status updated', {
        app_id: appId,
        customer_id: customer.id,
        retry_attempt: retryAttempt,
        next_retry_at: nextRetryAt,
        status: customerStatus
      });

      return updatedCustomer;
    } catch (error) {
      logger.error('Failed to update customer retry status', {
        app_id: appId,
        customer_id: customer.id,
        error: error.message
      });
      throw error;
    }
  }

  /**
   * Process retry for a customer (all active or past_due subscriptions)
   * @param {string} appId - Application identifier
   * @param {Object} customer - Customer record with subscriptions
   * @param {Object} config - Dunning configuration
   * @returns {Promise<Object>} Combined results for all subscriptions
   */
  async processCustomerRetry(appId, customer, config) {
    const { billing_subscriptions: subscriptions } = customer;
    // Include both 'active' and 'past_due' subscriptions for retry
    const retryableSubscriptions = subscriptions.filter(
      sub => sub.status === 'active' || sub.status === 'past_due'
    );

    if (retryableSubscriptions.length === 0) {
      logger.warn('No retryable subscriptions for customer retry', {
        app_id: appId,
        customer_id: customer.id
      });
      return {
        customerId: customer.id,
        processed: 0,
        successful: 0,
        failed: 0,
        results: []
      };
    }

    const results = [];
    let successfulRetries = 0;
    const successfulSubscriptionIds = [];

    for (const subscription of retryableSubscriptions) {
      const retryResult = await this.retrySubscriptionPayment(appId, customer, subscription);
      results.push({
        subscriptionId: subscription.id,
        tilledSubscriptionId: subscription.tilled_subscription_id,
        ...retryResult
      });

      if (retryResult.success) {
        successfulRetries++;
        successfulSubscriptionIds.push(subscription.id);
      }
    }

    // Update customer status based on overall retry results
    // If at least one subscription payment succeeded, consider it a partial success
    const overallSuccess = successfulRetries > 0;
    const combinedRetryResult = {
      success: overallSuccess,
      retryAttempt: customer.retry_attempt_count + 1,
      message: overallSuccess
        ? `${successfulRetries}/${retryableSubscriptions.length} subscription payments retried successfully`
        : `All ${retryableSubscriptions.length} subscription payment retries failed`
    };

    await this.updateCustomerRetryStatus(appId, customer, combinedRetryResult, config);

    // Update subscription statuses back to 'active' for successful retries
    if (successfulSubscriptionIds.length > 0) {
      await this.reactivateSubscriptions(appId, successfulSubscriptionIds);
    }

    return {
      customerId: customer.id,
      processed: retryableSubscriptions.length,
      successful: successfulRetries,
      failed: retryableSubscriptions.length - successfulRetries,
      results
    };
  }

  /**
   * Reactivate subscriptions after successful payment retry
   * @param {string} appId - Application identifier
   * @param {Array<number>} subscriptionIds - IDs of subscriptions to reactivate
   * @returns {Promise<void>}
   */
  async reactivateSubscriptions(appId, subscriptionIds) {
    try {
      const result = await billingPrisma.billing_subscriptions.updateMany({
        where: {
          id: { in: subscriptionIds },
          status: { in: ['past_due'] }
        },
        data: {
          status: 'active',
          updated_at: new Date()
        }
      });

      logger.info('Subscriptions reactivated after successful payment retry', {
        app_id: appId,
        subscription_ids: subscriptionIds,
        updated_count: result.count
      });
    } catch (error) {
      logger.error('Failed to reactivate subscriptions', {
        app_id: appId,
        subscription_ids: subscriptionIds,
        error: error.message
      });
      // Don't throw - customer status was already updated
    }
  }
}

module.exports = PaymentRetryService;