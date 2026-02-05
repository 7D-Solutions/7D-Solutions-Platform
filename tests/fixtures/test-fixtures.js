// Centralized test fixtures for billing module

const TEST_APPS = {
  trashtech: {
    app_id: 'trashtech',
    env_prefix: 'TRASHTECH'
  },
  apping: {
    app_id: 'apping',
    env_prefix: 'APPING'
  }
};

const TEST_CUSTOMERS = {
  standard: {
    app_id: 'trashtech',
    email: 'test@acmewaste.com',
    name: 'Acme Waste Inc',
    external_customer_id: '1',
    metadata: { industry: 'waste-management' }
  },
  noExternal: {
    app_id: 'trashtech',
    email: 'noexternal@example.com',
    name: 'No External ID Customer',
    external_customer_id: null,
    metadata: {}
  },
  apping: {
    app_id: 'apping',
    email: 'user@appingco.com',
    name: 'Apping User',
    external_customer_id: '42',
    metadata: { account_type: 'premium' }
  }
};

const TILLED_CUSTOMER_RESPONSE = {
  id: 'cus_test_123456',
  email: 'test@acmewaste.com',
  first_name: 'Acme Waste Inc',
  metadata: {}
};

const TILLED_PAYMENT_METHOD_RESPONSE = {
  id: 'pm_test_123456',
  type: 'card',
  card: {
    brand: 'visa',
    last4: '4242'
  }
};

const TILLED_PAYMENT_METHOD_ACH = {
  id: 'pm_test_ach_123456',
  type: 'ach_debit',
  ach_debit: {
    bank_name: 'Test Bank',
    last4: '6789'
  }
};

const TILLED_SUBSCRIPTION_RESPONSE = {
  id: 'sub_test_123456',
  status: 'active',
  customer_id: 'cus_test_123456',
  payment_method_id: 'pm_test_123456',
  price: 9900,
  currency: 'usd',
  interval_unit: 'month',
  interval_count: 1,
  billing_cycle_anchor: Math.floor(Date.now() / 1000),
  current_period_start: Math.floor(Date.now() / 1000),
  current_period_end: Math.floor((Date.now() + 30 * 24 * 60 * 60 * 1000) / 1000),
  cancel_at: null,
  canceled_at: null,
  metadata: {}
};

const TEST_SUBSCRIPTIONS = {
  monthly: {
    billing_customer_id: 1,
    payment_method_id: 'pm_test_123456',
    plan_id: 'pro-monthly',
    plan_name: 'Pro Monthly',
    price_cents: 9900,
    options: {
      intervalUnit: 'month',
      intervalCount: 1,
      metadata: { features: ['unlimited_routes', 'analytics'] }
    }
  },
  annual: {
    billing_customer_id: 1,
    payment_method_id: 'pm_test_123456',
    plan_id: 'pro-annual',
    plan_name: 'Pro Annual',
    price_cents: 99000,
    options: {
      intervalUnit: 'year',
      intervalCount: 1,
      metadata: { features: ['unlimited_routes', 'analytics'] }
    }
  },
  ach: {
    billing_customer_id: 1,
    payment_method_id: 'pm_test_ach_123456',
    plan_id: 'pro-monthly-ach',
    plan_name: 'Pro Monthly (ACH)',
    price_cents: 9900,
    options: {
      intervalUnit: 'month',
      intervalCount: 1,
      metadata: { payment_type: 'ach' }
    }
  }
};

const WEBHOOK_EVENTS = {
  subscriptionCreated: {
    id: 'evt_test_created_123',
    type: 'subscription.created',
    data: {
      object: TILLED_SUBSCRIPTION_RESPONSE
    }
  },
  subscriptionUpdated: {
    id: 'evt_test_updated_123',
    type: 'subscription.updated',
    data: {
      object: {
        ...TILLED_SUBSCRIPTION_RESPONSE,
        status: 'past_due'
      }
    }
  },
  subscriptionCanceled: {
    id: 'evt_test_canceled_123',
    type: 'subscription.canceled',
    data: {
      object: {
        ...TILLED_SUBSCRIPTION_RESPONSE,
        status: 'canceled',
        canceled_at: Math.floor(Date.now() / 1000)
      }
    }
  }
};

// Helper to generate webhook signature
function generateWebhookSignature(payload, secret, timestamp = Math.floor(Date.now() / 1000)) {
  const crypto = require('crypto');
  const signedPayload = `${timestamp}.${JSON.stringify(payload)}`;
  const signature = crypto
    .createHmac('sha256', secret)
    .update(signedPayload)
    .digest('hex');
  return `t=${timestamp},v1=${signature}`;
}

module.exports = {
  TEST_APPS,
  TEST_CUSTOMERS,
  TILLED_CUSTOMER_RESPONSE,
  TILLED_PAYMENT_METHOD_RESPONSE,
  TILLED_PAYMENT_METHOD_ACH,
  TILLED_SUBSCRIPTION_RESPONSE,
  TEST_SUBSCRIPTIONS,
  WEBHOOK_EVENTS,
  generateWebhookSignature
};
