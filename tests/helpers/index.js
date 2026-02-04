// Test helpers for billing module

/**
 * Create mock Tilled API responses
 */
function mockTilledAPI() {
  return {
    customersApi: {
      createCustomer: jest.fn()
    },
    subscriptionsApi: {
      createSubscription: jest.fn(),
      cancelSubscription: jest.fn()
    },
    paymentMethodsApi: {
      attachPaymentMethodToCustomer: jest.fn()
    }
  };
}

/**
 * Clean database tables for testing
 */
async function cleanDatabase(billingPrisma) {
  await billingPrisma.billing_webhooks.deleteMany({});
  await billingPrisma.billing_subscriptions.deleteMany({});
  await billingPrisma.billing_customers.deleteMany({});
}

/**
 * Create test customer in database
 */
async function createTestCustomer(billingPrisma, customerData) {
  return billingPrisma.billing_customers.create({
    data: {
      app_id: customerData.app_id,
      external_customer_id: customerData.external_customer_id,
      tilled_customer_id: customerData.tilled_customer_id || 'cus_test_123',
      email: customerData.email,
      name: customerData.name,
      metadata: customerData.metadata || {}
    }
  });
}

/**
 * Create test subscription in database
 */
async function createTestSubscription(billingPrisma, subscriptionData) {
  return billingPrisma.billing_subscriptions.create({
    data: {
      app_id: subscriptionData.app_id,
      billing_customer_id: subscriptionData.billing_customer_id,
      tilled_subscription_id: subscriptionData.tilled_subscription_id || 'sub_test_123',
      plan_id: subscriptionData.plan_id,
      plan_name: subscriptionData.plan_name,
      price_cents: subscriptionData.price_cents,
      status: subscriptionData.status || 'active',
      interval_unit: subscriptionData.interval_unit || 'month',
      interval_count: subscriptionData.interval_count || 1,
      current_period_start: subscriptionData.current_period_start || new Date(),
      current_period_end: subscriptionData.current_period_end || new Date(Date.now() + 30 * 24 * 60 * 60 * 1000),
      payment_method_id: subscriptionData.payment_method_id,
      payment_method_type: subscriptionData.payment_method_type || 'card',
      metadata: subscriptionData.metadata || {}
    }
  });
}

/**
 * Wait for a condition to be true (with timeout)
 */
async function waitFor(condition, timeout = 5000, interval = 100) {
  const startTime = Date.now();
  while (Date.now() - startTime < timeout) {
    if (await condition()) {
      return true;
    }
    await new Promise(resolve => setTimeout(resolve, interval));
  }
  throw new Error(`Timeout waiting for condition after ${timeout}ms`);
}

module.exports = {
  mockTilledAPI,
  cleanDatabase,
  createTestCustomer,
  createTestSubscription,
  waitFor
};
