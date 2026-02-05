const BillingStateService = require('../../backend/src/services/BillingStateService');
const { billingPrisma } = require('../../backend/src/prisma');
const TilledClient = require('../../backend/src/tilledClient');

// Mock dependencies
jest.mock('../../backend/src/tilledClient');
jest.mock('../../backend/src/prisma', () => ({
  billingPrisma: {
    billing_customers: {
      findFirst: jest.fn(),
      findUnique: jest.fn()
    },
    billing_subscriptions: {
      findFirst: jest.fn(),
      findMany: jest.fn()
    },
    billing_payment_methods: {
      findFirst: jest.fn()
    }
  }
}));

describe('BillingStateService.getBillingState', () => {
  let service;
  let mockTilledClient;

  beforeEach(() => {
    service = new BillingStateService();
    mockTilledClient = {
      getPaymentMethod: jest.fn()
    };
    TilledClient.mockImplementation(() => mockTilledClient);
    jest.clearAllMocks();
  });

  it('throws if customer not found for app+external_customer_id', async () => {
    billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

    await expect(
      service.getBillingState('trashtech', '123')
    ).rejects.toThrow('Customer with external_customer_id 123 not found for app trashtech');
  });

  it('returns active subscription when one exists', async () => {
    const mockCustomer = {
      id: 1,
      app_id: 'trashtech',
      external_customer_id: '123',
      email: 'test@example.com',
      name: 'Test Customer',
      default_payment_method_id: 'pm_test',
      payment_method_type: 'card'
    };

    const mockActiveSubscription = {
      id: 10,
      billing_customer_id: 1,
      plan_id: 'pro-monthly',
      plan_name: 'Pro Monthly',
      price_cents: 9900,
      status: 'active',
      current_period_start: new Date('2026-01-01'),
      current_period_end: new Date('2026-02-01'),
      cancel_at_period_end: false,
      canceled_at: null,
      ended_at: null,
      metadata: {}
    };

    const mockInactiveSubscription = {
      id: 9,
      status: 'canceled',
      created_at: new Date('2025-12-01')
    };

    const mockDefaultPM = {
      tilled_payment_method_id: 'pm_test',
      type: 'card',
      brand: 'visa',
      last4: '4242',
      exp_month: 12,
      exp_year: 2028,
      is_default: true,
      deleted_at: null
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_subscriptions.findMany.mockResolvedValue([
      mockActiveSubscription,
      mockInactiveSubscription
    ]);
    billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(mockDefaultPM);

    // Set env var for entitlements
    process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH = JSON.stringify({
      'pro-monthly': { analytics: true, max_trucks: 10 }
    });

    const result = await service.getBillingState('trashtech', '123');

    expect(result.customer).toMatchObject({
      id: 1,
      email: 'test@example.com',
      external_customer_id: '123'
    });

    expect(result.subscription).toMatchObject({
      id: 10,
      status: 'active',
      plan_id: 'pro-monthly'
    });

    expect(result.payment.has_default_payment_method).toBe(true);
    expect(result.payment.default_payment_method).toMatchObject({
      id: 'pm_test',
      type: 'card',
      brand: 'visa',
      last4: '4242'
    });

    expect(result.access.is_active).toBe(true);
    expect(result.access.access_state).toBe('full');

    expect(result.entitlements.plan_id).toBe('pro-monthly');
    expect(result.entitlements.features).toEqual({
      analytics: true,
      max_trucks: 10
    });

    delete process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH;
  });

  it('falls back to most recent subscription when no active', async () => {
    const mockCustomer = {
      id: 1,
      app_id: 'trashtech',
      external_customer_id: '123',
      email: 'test@example.com'
    };

    const mockOldSubscription = {
      id: 9,
      status: 'canceled',
      plan_id: 'basic',
      created_at: new Date('2025-12-01')
    };

    const mockRecentSubscription = {
      id: 10,
      status: 'canceled',
      plan_id: 'pro-monthly',
      created_at: new Date('2026-01-01')
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_subscriptions.findMany.mockResolvedValue([
      mockRecentSubscription,
      mockOldSubscription
    ]);
    billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(null);

    const result = await service.getBillingState('trashtech', '123');

    expect(result.subscription.id).toBe(10);
    expect(result.subscription.status).toBe('canceled');
    expect(result.access.is_active).toBe(false);
    expect(result.access.access_state).toBe('locked');
  });

  it('returns default PM using billing_customers fast-path when present', async () => {
    const mockCustomer = {
      id: 1,
      app_id: 'trashtech',
      external_customer_id: '123',
      email: 'test@example.com',
      default_payment_method_id: 'pm_fast',
      payment_method_type: 'card'
    };

    const mockDefaultPM = {
      tilled_payment_method_id: 'pm_fast',
      type: 'card',
      brand: 'visa',
      last4: '4242',
      exp_month: 12,
      exp_year: 2028,
      is_default: true,
      deleted_at: null
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_subscriptions.findMany.mockResolvedValue([]);
    billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(mockDefaultPM);

    const result = await service.getBillingState('trashtech', '123');

    expect(billingPrisma.billing_payment_methods.findFirst).toHaveBeenCalledWith({
      where: {
        billing_customer_id: 1,
        tilled_payment_method_id: 'pm_fast',
        deleted_at: null
      }
    });

    expect(result.payment.has_default_payment_method).toBe(true);
    expect(result.payment.default_payment_method.id).toBe('pm_fast');
  });

  it('falls back to is_default payment method when fast-path missing', async () => {
    const mockCustomer = {
      id: 1,
      app_id: 'trashtech',
      external_customer_id: '123',
      email: 'test@example.com',
      default_payment_method_id: 'pm_missing',  // Has ID but PM doesn't exist
      payment_method_type: 'card'
    };

    const mockDefaultPM = {
      tilled_payment_method_id: 'pm_fallback',
      type: 'card',
      brand: 'mastercard',
      last4: '5555',
      exp_month: 6,
      exp_year: 2027,
      is_default: true,
      deleted_at: null
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_subscriptions.findMany.mockResolvedValue([]);

    // First call (fast-path) returns null, second call (fallback) returns method
    billingPrisma.billing_payment_methods.findFirst
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce(mockDefaultPM);

    const result = await service.getBillingState('trashtech', '123');

    expect(billingPrisma.billing_payment_methods.findFirst).toHaveBeenCalledTimes(2);
    expect(billingPrisma.billing_payment_methods.findFirst).toHaveBeenNthCalledWith(2, {
      where: {
        billing_customer_id: 1,
        is_default: true,
        deleted_at: null
      }
    });

    expect(result.payment.has_default_payment_method).toBe(true);
    expect(result.payment.default_payment_method.id).toBe('pm_fallback');
  });

  it('computes access_state full when active subscription, locked otherwise', async () => {
    const mockCustomer = {
      id: 1,
      app_id: 'trashtech',
      external_customer_id: '123',
      email: 'test@example.com'
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(null);

    // Test with active subscription
    billingPrisma.billing_subscriptions.findMany.mockResolvedValue([{
      id: 1,
      status: 'active',
      plan_id: 'pro'
    }]);

    let result = await service.getBillingState('trashtech', '123');
    expect(result.access.access_state).toBe('full');
    expect(result.access.is_active).toBe(true);

    // Test with canceled subscription
    billingPrisma.billing_subscriptions.findMany.mockResolvedValue([{
      id: 1,
      status: 'canceled',
      plan_id: 'pro'
    }]);

    result = await service.getBillingState('trashtech', '123');
    expect(result.access.access_state).toBe('locked');
    expect(result.access.is_active).toBe(false);
  });

  it('computes entitlements from env JSON map + merges features_overrides', async () => {
    const mockCustomer = {
      id: 1,
      app_id: 'trashtech',
      external_customer_id: '123'
    };

    const mockSubscription = {
      id: 1,
      status: 'active',
      plan_id: 'pro-monthly',
      metadata: {
        features_overrides: {
          max_trucks: 20,  // Override from 10 to 20
          custom_feature: true  // Additional feature
        }
      }
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_subscriptions.findMany.mockResolvedValue([mockSubscription]);
    billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(null);

    process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH = JSON.stringify({
      'pro-monthly': { analytics: true, max_trucks: 10 }
    });

    const result = await service.getBillingState('trashtech', '123');

    expect(result.entitlements.features).toEqual({
      analytics: true,
      max_trucks: 20,  // Overridden
      custom_feature: true  // Added
    });

    delete process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH;
  });

  it('handles malformed entitlements JSON by returning empty features', async () => {
    const mockCustomer = {
      id: 1,
      app_id: 'trashtech',
      external_customer_id: '123'
    };

    const mockSubscription = {
      id: 1,
      status: 'active',
      plan_id: 'pro-monthly'
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_subscriptions.findMany.mockResolvedValue([mockSubscription]);
    billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(null);

    // Set malformed JSON
    process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH = '{invalid json';

    const result = await service.getBillingState('trashtech', '123');

    expect(result.entitlements.features).toEqual({});

    delete process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH;
  });

  it('handles missing plan_id in entitlements map by returning empty features', async () => {
    const mockCustomer = {
      id: 1,
      app_id: 'trashtech',
      external_customer_id: '123'
    };

    const mockSubscription = {
      id: 1,
      status: 'active',
      plan_id: 'unknown-plan'
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_subscriptions.findMany.mockResolvedValue([mockSubscription]);
    billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(null);

    process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH = JSON.stringify({
      'pro-monthly': { analytics: true }
    });

    const result = await service.getBillingState('trashtech', '123');

    expect(result.entitlements.plan_id).toBe('unknown-plan');
    expect(result.entitlements.features).toEqual({});

    delete process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH;
  });
});

describe('BillingStateService.getEntitlements', () => {
  let service;

  beforeEach(() => {
    service = new BillingStateService();
    jest.clearAllMocks();
  });

  it('returns null plan_id and empty features when no subscription', () => {
    const result = service.getEntitlements('trashtech', null);

    expect(result).toEqual({
      plan_id: null,
      features: {}
    });
  });

  it('returns plan features from env map', () => {
    process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH = JSON.stringify({
      'pro-monthly': { analytics: true, max_trucks: 10 }
    });

    const mockSubscription = {
      plan_id: 'pro-monthly',
      metadata: {}
    };

    const result = service.getEntitlements('trashtech', mockSubscription);

    expect(result.plan_id).toBe('pro-monthly');
    expect(result.features).toEqual({
      analytics: true,
      max_trucks: 10
    });

    delete process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH;
  });

  it('merges features_overrides from subscription metadata', () => {
    process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH = JSON.stringify({
      'pro-monthly': { analytics: true, max_trucks: 10 }
    });

    const mockSubscription = {
      plan_id: 'pro-monthly',
      metadata: {
        features_overrides: {
          max_trucks: 50,
          unlimited_storage: true
        }
      }
    };

    const result = service.getEntitlements('trashtech', mockSubscription);

    expect(result.features).toEqual({
      analytics: true,
      max_trucks: 50,  // Overridden
      unlimited_storage: true  // Added
    });

    delete process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH;
  });

  it('returns empty features when env var missing', () => {
    const mockSubscription = {
      plan_id: 'pro-monthly',
      metadata: {}
    };

    const result = service.getEntitlements('trashtech', mockSubscription);

    expect(result.features).toEqual({});
  });

  it('handles malformed JSON gracefully', () => {
    process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH = '{invalid}';

    const mockSubscription = {
      plan_id: 'pro-monthly',
      metadata: {}
    };

    const result = service.getEntitlements('trashtech', mockSubscription);

    expect(result.features).toEqual({});

    delete process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH;
  });
});
