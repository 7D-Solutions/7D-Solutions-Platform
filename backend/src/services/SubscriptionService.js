const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError, ConflictError, UnauthorizedError } = require('../utils/errors');

class SubscriptionService {
  constructor(getTilledClientFn) {
    this.getTilledClient = getTilledClientFn;
  }

  async createSubscription(appId, billingCustomerId, paymentMethodId, planId, planName, priceCents, options = {}) {
    const billingCustomer = await billingPrisma.billing_customers.findFirst({
      where: {
        id: billingCustomerId,
        app_id: appId
      }
    });

    if (!billingCustomer) {
      throw new NotFoundError(`Billing customer ${billingCustomerId} not found for app ${appId}`);
    }

    const tilledClient = this.getTilledClient(billingCustomer.app_id);

    // Step 1: Attach payment method to customer
    const paymentMethod = await tilledClient.attachPaymentMethod(
      paymentMethodId,
      billingCustomer.tilled_customer_id
    );

    // Step 2: Create subscription in Tilled
    const tilledSubscription = await tilledClient.createSubscription(
      billingCustomer.tilled_customer_id,
      paymentMethodId,
      priceCents,
      options
    );

    // Step 3: Save to database
    const subscription = await billingPrisma.billing_subscriptions.create({
      data: {
        app_id: billingCustomer.app_id,
        billing_customer_id: billingCustomerId,
        tilled_subscription_id: tilledSubscription.id,
        plan_id: planId,
        plan_name: planName,
        price_cents: priceCents,
        status: tilledSubscription.status,
        interval_unit: options.intervalUnit || 'month',
        interval_count: options.intervalCount || 1,
        billing_cycle_anchor: tilledSubscription.billing_cycle_anchor ?
          new Date(tilledSubscription.billing_cycle_anchor * 1000) : null,
        current_period_start: new Date(tilledSubscription.current_period_start * 1000),
        current_period_end: new Date(tilledSubscription.current_period_end * 1000),
        cancel_at: tilledSubscription.cancel_at ? new Date(tilledSubscription.cancel_at * 1000) : null,
        canceled_at: tilledSubscription.canceled_at ? new Date(tilledSubscription.canceled_at * 1000) : null,
        payment_method_id: paymentMethodId,
        payment_method_type: paymentMethod.type || 'card',
        metadata: options.metadata || {}
      }
    });

    // Audit trail (fire-and-forget)
    billingPrisma.billing_events?.create({
      data: {
        app_id: appId,
        event_type: 'subscription.created',
        source: 'subscription_service',
        entity_type: 'subscription',
        entity_id: String(subscription.id),
        payload: {
          subscription_id: subscription.id,
          tilled_subscription_id: tilledSubscription.id,
          billing_customer_id: billingCustomerId,
          plan_id: planId,
          plan_name: planName,
          price_cents: priceCents,
          interval_unit: options.intervalUnit || 'month',
          interval_count: options.intervalCount || 1,
        },
      },
    })?.catch(err => logger.warn('Failed to record subscription audit event', { error: err.message }));

    return subscription;
  }

  async cancelSubscription(subscriptionId) {
    const subscription = await billingPrisma.billing_subscriptions.findUnique({
      where: { id: subscriptionId }
    });

    if (!subscription) throw new NotFoundError(`Subscription ${subscriptionId} not found`);

    const customer = await billingPrisma.billing_customers.findUnique({
      where: { id: subscription.billing_customer_id }
    });

    const tilledClient = this.getTilledClient(customer.app_id);
    const tilledSubscription = await tilledClient.cancelSubscription(subscription.tilled_subscription_id);

    return billingPrisma.billing_subscriptions.update({
      where: { id: subscriptionId },
      data: {
        status: tilledSubscription.status,
        canceled_at: tilledSubscription.canceled_at ? new Date(tilledSubscription.canceled_at * 1000) : null,
        updated_at: new Date()
      }
    });
  }

  async cancelSubscriptionEx(appId, subscriptionId, options = {}) {
    // Verify subscription belongs to app
    const subscription = await billingPrisma.billing_subscriptions.findFirst({
      where: { id: subscriptionId },
      include: { billing_customers: true }
    });

    if (!subscription || subscription.billing_customers.app_id !== appId) {
      throw new NotFoundError(`Subscription ${subscriptionId} not found for app ${appId}`);
    }

    const tilledClient = this.getTilledClient(appId);
    const atPeriodEnd = options.atPeriodEnd || false;

    if (atPeriodEnd) {
      // Set cancel_at_period_end flag without immediate cancellation
      try {
        await tilledClient.updateSubscription(subscription.tilled_subscription_id, {
          cancel_at_period_end: true
        });
      } catch (error) {
        logger.warn('Failed to set cancel_at_period_end in Tilled', {
          app_id: appId,
          subscription_id: subscriptionId,
          error_message: error.message
        });
      }

      const updated = await billingPrisma.billing_subscriptions.update({
        where: { id: subscriptionId },
        data: {
          cancel_at_period_end: true,
          updated_at: new Date()
        }
      });

      // Audit trail (fire-and-forget)
      billingPrisma.billing_events?.create({
        data: {
          app_id: appId,
          event_type: 'subscription.cancel_scheduled',
          source: 'subscription_service',
          entity_type: 'subscription',
          entity_id: String(subscriptionId),
          payload: {
            subscription_id: subscriptionId,
            billing_customer_id: subscription.billing_customer_id,
            plan_id: subscription.plan_id,
            cancel_at_period_end: true,
          },
        },
      })?.catch(err => logger.warn('Failed to record subscription audit event', { error: err.message }));

      return updated;
    } else {
      // Immediate cancellation
      const tilledSubscription = await tilledClient.cancelSubscription(subscription.tilled_subscription_id);

      const updated = await billingPrisma.billing_subscriptions.update({
        where: { id: subscriptionId },
        data: {
          status: 'canceled',
          cancel_at_period_end: false,
          canceled_at: tilledSubscription.canceled_at ? new Date(tilledSubscription.canceled_at * 1000) : new Date(),
          ended_at: new Date(),
          updated_at: new Date()
        }
      });

      // Audit trail (fire-and-forget)
      billingPrisma.billing_events?.create({
        data: {
          app_id: appId,
          event_type: 'subscription.canceled',
          source: 'subscription_service',
          entity_type: 'subscription',
          entity_id: String(subscriptionId),
          payload: {
            subscription_id: subscriptionId,
            billing_customer_id: subscription.billing_customer_id,
            plan_id: subscription.plan_id,
            old_status: subscription.status,
            new_status: 'canceled',
          },
        },
      })?.catch(err => logger.warn('Failed to record subscription audit event', { error: err.message }));

      return updated;
    }
  }

  async changeCycle(appId, payload) {
    const {
      billing_customer_id,
      from_subscription_id,
      new_plan_id,
      new_plan_name,
      price_cents,
      payment_method_id,
      payment_method_type,
      options = {}
    } = payload;

    // Validate required fields
    if (!billing_customer_id || !from_subscription_id || !new_plan_id || !new_plan_name || !price_cents || !payment_method_id) {
      throw new ValidationError('Missing required fields for cycle change');
    }

    // Step 1: Verify customer belongs to app
    const customer = await billingPrisma.billing_customers.findFirst({
      where: {
        id: billing_customer_id,
        app_id: appId
      }
    });

    if (!customer) {
      throw new NotFoundError(`Customer ${billing_customer_id} not found for app ${appId}`);
    }

    // Step 2: Verify old subscription belongs to customer
    const oldSubscription = await billingPrisma.billing_subscriptions.findFirst({
      where: {
        id: from_subscription_id,
        billing_customer_id: billing_customer_id
      },
      include: { billing_customers: true }
    });

    if (!oldSubscription || oldSubscription.billing_customers.app_id !== appId) {
      throw new NotFoundError(`Subscription ${from_subscription_id} not found for app ${appId}`);
    }

    const tilledClient = this.getTilledClient(appId);

    // Step 3: Attach payment method if needed
    await tilledClient.attachPaymentMethod(payment_method_id, customer.tilled_customer_id);

    // Step 4: Create new subscription in Tilled
    const newTilledSubscription = await tilledClient.createSubscription(
      customer.tilled_customer_id,
      payment_method_id,
      price_cents,
      {
        intervalUnit: options.intervalUnit,
        intervalCount: options.intervalCount,
        metadata: options.metadata || {}
      }
    );

    // Step 5: Cancel old subscription in Tilled
    const canceledTilledSub = await tilledClient.cancelSubscription(oldSubscription.tilled_subscription_id);

    // Step 6: Persist both changes in database transaction
    return billingPrisma.$transaction(async (tx) => {
      // Cancel old subscription
      const canceledSubscription = await tx.billing_subscriptions.update({
        where: { id: from_subscription_id },
        data: {
          status: 'canceled',
          canceled_at: canceledTilledSub.canceled_at ? new Date(canceledTilledSub.canceled_at * 1000) : new Date(),
          ended_at: new Date(),
          updated_at: new Date()
        }
      });

      // Create new subscription
      const newSubscription = await tx.billing_subscriptions.create({
        data: {
          app_id: appId,
          billing_customer_id: billing_customer_id,
          tilled_subscription_id: newTilledSubscription.id,
          plan_id: new_plan_id,
          plan_name: new_plan_name,
          price_cents: price_cents,
          status: newTilledSubscription.status,
          interval_unit: options.intervalUnit || 'month',
          interval_count: options.intervalCount || 1,
          billing_cycle_anchor: newTilledSubscription.billing_cycle_anchor ?
            new Date(newTilledSubscription.billing_cycle_anchor * 1000) : null,
          current_period_start: new Date(newTilledSubscription.current_period_start * 1000),
          current_period_end: new Date(newTilledSubscription.current_period_end * 1000),
          cancel_at: newTilledSubscription.cancel_at ? new Date(newTilledSubscription.cancel_at * 1000) : null,
          canceled_at: newTilledSubscription.canceled_at ? new Date(newTilledSubscription.canceled_at * 1000) : null,
          payment_method_id: payment_method_id,
          payment_method_type: payment_method_type || 'card',
          metadata: options.metadata || {}
        }
      });

      return {
        canceled_subscription: canceledSubscription,
        new_subscription: newSubscription
      };
    });
  }

  async getSubscriptionById(appId, subscriptionId) {
    const subscription = await billingPrisma.billing_subscriptions.findFirst({
      where: { id: subscriptionId },
      include: { billing_customers: true }
    });

    if (!subscription) throw new NotFoundError(`Subscription ${subscriptionId} not found`);

    // Verify belongs to app via customer
    if (subscription.billing_customers.app_id !== appId) {
      throw new NotFoundError(`Subscription ${subscriptionId} not found for app ${appId}`);
    }

    return subscription;
  }

  async listSubscriptions(filters = {}) {
    const where = {};

    if (filters.billingCustomerId) {
      where.billing_customer_id = filters.billingCustomerId;
    }

    if (filters.status) {
      where.status = filters.status;
    }

    // If filtering by app_id, need to join through customer
    if (filters.appId) {
      where.billing_customers = {
        app_id: filters.appId
      };
    }

    return billingPrisma.billing_subscriptions.findMany({
      where,
      include: { billing_customers: true },
      orderBy: { created_at: 'desc' }
    });
  }

  async updateSubscription(appId, subscriptionId, patch) {
    // Verify subscription belongs to app
    const subscription = await this.getSubscriptionById(appId, subscriptionId);

    // CRITICAL: Detect billing cycle changes (not allowed by Tilled)
    const cycleFields = ['interval_unit', 'interval_count', 'billing_cycle_anchor'];
    const hasCycleChange = cycleFields.some(field => patch[field] !== undefined);

    if (hasCycleChange) {
      throw new ValidationError('Cannot change billing cycle after subscription creation. Use cancel+create pattern instead.');
    }

    // Extract allowed fields and detect unsupported fields
    const allowedFields = ['plan_id', 'plan_name', 'price_cents', 'metadata'];
    const providedFields = Object.keys(patch);
    const unsupportedFields = providedFields.filter(field => !allowedFields.includes(field));

    if (unsupportedFields.length > 0) {
      throw new ValidationError(`Unsupported field(s): ${unsupportedFields.join(', ')}`);
    }

    const updates = {};
    allowedFields.forEach(field => {
      if (patch[field] !== undefined) {
        updates[field] = patch[field];
      }
    });

    if (Object.keys(updates).length === 0) {
      throw new ValidationError('No valid fields to update');
    }

    updates.updated_at = new Date();

    // Update in database
    const updatedSubscription = await billingPrisma.billing_subscriptions.update({
      where: { id: subscriptionId },
      data: updates
    });

    // Sync metadata to Tilled if changed
    if (patch.metadata) {
      try {
        const tilledClient = this.getTilledClient(appId);
        await tilledClient.updateSubscription(subscription.tilled_subscription_id, {
          metadata: patch.metadata
        });
      } catch (error) {
        // CRITICAL: Log enough to reconcile later
        logger.warn('Failed to sync subscription metadata to Tilled', {
          app_id: appId,
          billing_subscription_id: subscriptionId,
          tilled_subscription_id: subscription.tilled_subscription_id,
          attempted_updates: Object.keys(patch),
          error_message: error.message,
          error_code: error.code
        });
      }
    }

    return updatedSubscription;
  }
}

module.exports = SubscriptionService;
