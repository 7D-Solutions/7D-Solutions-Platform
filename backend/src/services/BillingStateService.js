const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError, ConflictError, UnauthorizedError } = require('../utils/errors');

class BillingStateService {
  async getBillingState(appId, externalCustomerId) {
    // Step 1: Find customer by app + external_customer_id
    const customer = await billingPrisma.billing_customers.findFirst({
      where: {
        app_id: appId,
        external_customer_id: String(externalCustomerId)
      }
    });

    if (!customer) {
      throw new NotFoundError(`Customer with external_customer_id ${externalCustomerId} not found for app ${appId}`);
    }

    // Step 2: Find active subscription or most recent
    const subscriptions = await billingPrisma.billing_subscriptions.findMany({
      where: { billing_customer_id: customer.id },
      orderBy: { created_at: 'desc' }
    });

    const activeSubscription = subscriptions.find(sub => sub.status === 'active');
    const subscription = activeSubscription || subscriptions[0] || null;

    // Step 3: Get default payment method (fast-path first, fallback to is_default)
    let defaultPaymentMethod = null;
    if (customer.default_payment_method_id) {
      defaultPaymentMethod = await billingPrisma.billing_payment_methods.findFirst({
        where: {
          billing_customer_id: customer.id,
          tilled_payment_method_id: customer.default_payment_method_id,
          deleted_at: null
        }
      });
    }

    // Fallback to is_default flag
    if (!defaultPaymentMethod) {
      defaultPaymentMethod = await billingPrisma.billing_payment_methods.findFirst({
        where: {
          billing_customer_id: customer.id,
          is_default: true,
          deleted_at: null
        }
      });
    }

    // Step 4: Compute access state
    const isActive = subscription && subscription.status === 'active';
    const accessState = isActive ? 'full' : 'locked';

    // Step 5: Compute entitlements
    const entitlements = this.getEntitlements(appId, subscription);

    // Step 6: Compose response
    return {
      customer: {
        id: customer.id,
        email: customer.email,
        name: customer.name,
        external_customer_id: customer.external_customer_id,
        metadata: customer.metadata
      },
      subscription: subscription ? {
        id: subscription.id,
        plan_id: subscription.plan_id,
        plan_name: subscription.plan_name,
        price_cents: subscription.price_cents,
        status: subscription.status,
        interval_unit: subscription.interval_unit,
        interval_count: subscription.interval_count,
        current_period_start: subscription.current_period_start,
        current_period_end: subscription.current_period_end,
        cancel_at_period_end: subscription.cancel_at_period_end,
        canceled_at: subscription.canceled_at,
        ended_at: subscription.ended_at,
        metadata: subscription.metadata
      } : null,
      payment: {
        has_default_payment_method: !!defaultPaymentMethod,
        default_payment_method: defaultPaymentMethod ? {
          id: defaultPaymentMethod.tilled_payment_method_id,
          type: defaultPaymentMethod.type,
          brand: defaultPaymentMethod.brand,
          last4: defaultPaymentMethod.last4,
          exp_month: defaultPaymentMethod.exp_month,
          exp_year: defaultPaymentMethod.exp_year,
          bank_name: defaultPaymentMethod.bank_name,
          bank_last4: defaultPaymentMethod.bank_last4
        } : null
      },
      access: {
        is_active: isActive,
        access_state: accessState
      },
      entitlements
    };
  }

  getEntitlements(appId, subscription) {
    if (!subscription) {
      return {
        plan_id: null,
        features: {}
      };
    }

    // Load entitlements from environment variable
    const envKey = `BILLING_ENTITLEMENTS_JSON_${appId.toUpperCase()}`;
    const entitlementsJson = process.env[envKey];

    let planFeatures = {};

    if (entitlementsJson) {
      try {
        const entitlementsMap = JSON.parse(entitlementsJson);
        planFeatures = entitlementsMap[subscription.plan_id] || {};
      } catch (error) {
        logger.warn('Failed to parse entitlements JSON', {
          app_id: appId,
          env_key: envKey,
          error_message: error.message
        });
      }
    }

    // Merge with subscription metadata overrides
    const overrides = subscription.metadata?.features_overrides || {};
    const mergedFeatures = { ...planFeatures, ...overrides };

    return {
      plan_id: subscription.plan_id,
      features: mergedFeatures
    };
  }
}

module.exports = BillingStateService;
