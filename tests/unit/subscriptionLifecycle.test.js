const BillingService = require('../../backend/src/billingService');
const { billingPrisma } = require('../../backend/src/prisma');
const TilledClient = require('../../backend/src/tilledClient');

// Mock dependencies
jest.mock('../../backend/src/tilledClient');
jest.mock('../../backend/src/prisma', () => ({
  billingPrisma: {
    billing_customers: {
      findUnique: jest.fn(),
      findFirst: jest.fn()
    },
    billing_subscriptions: {
      findUnique: jest.fn(),
      findFirst: jest.fn(),
      update: jest.fn(),
      create: jest.fn()
    },
    $transaction: jest.fn()
  }
}));

describe('BillingService.cancelSubscriptionEx', () => {
  let service;
  let mockTilledClient;

  beforeEach(() => {
    service = new BillingService();
    mockTilledClient = {
      updateSubscription: jest.fn(),
      cancelSubscription: jest.fn()
    };
    TilledClient.mockImplementation(() => mockTilledClient);
    jest.clearAllMocks();
  });

  it('sets cancel_at_period_end=true without canceling immediately when atPeriodEnd=true', async () => {
    const mockSubscription = {
      id: 10,
      billing_customer_id: 1,
      tilled_subscription_id: 'sub_test',
      status: 'active',
      cancel_at_period_end: false
    };

    const mockCustomer = {
      id: 1,
      app_id: 'trashtech'
    };

    const mockUpdatedSubscription = {
      ...mockSubscription,
      cancel_at_period_end: true
    };

    billingPrisma.billing_subscriptions.findFirst.mockResolvedValue({
      ...mockSubscription,
      billing_customers: mockCustomer
    });
    mockTilledClient.updateSubscription.mockResolvedValue({ cancel_at_period_end: true });
    billingPrisma.billing_subscriptions.update.mockResolvedValue(mockUpdatedSubscription);

    const result = await service.cancelSubscriptionEx('trashtech', 10, { atPeriodEnd: true });

    expect(billingPrisma.billing_subscriptions.update).toHaveBeenCalledWith({
      where: { id: 10 },
      data: {
        cancel_at_period_end: true,
        updated_at: expect.any(Date)
      }
    });

    expect(mockTilledClient.updateSubscription).toHaveBeenCalledWith(
      'sub_test',
      { cancel_at_period_end: true }
    );

    expect(result.cancel_at_period_end).toBe(true);
    expect(result.status).toBe('active');  // Remains active
  });

  it('immediate cancel sets status=canceled, canceled_at, ended_at when atPeriodEnd=false', async () => {
    const mockSubscription = {
      id: 10,
      billing_customer_id: 1,
      tilled_subscription_id: 'sub_test',
      status: 'active',
      cancel_at_period_end: false
    };

    const mockCustomer = {
      id: 1,
      app_id: 'trashtech'
    };

    const mockTilledCanceledSub = {
      id: 'sub_test',
      status: 'canceled',
      canceled_at: Math.floor(Date.now() / 1000)
    };

    billingPrisma.billing_subscriptions.findFirst.mockResolvedValue({
      ...mockSubscription,
      billing_customers: mockCustomer
    });
    mockTilledClient.cancelSubscription.mockResolvedValue(mockTilledCanceledSub);
    billingPrisma.billing_subscriptions.update.mockResolvedValue({
      ...mockSubscription,
      status: 'canceled',
      canceled_at: new Date(),
      ended_at: new Date()
    });

    const result = await service.cancelSubscriptionEx('trashtech', 10, { atPeriodEnd: false });

    expect(mockTilledClient.cancelSubscription).toHaveBeenCalledWith('sub_test');

    expect(billingPrisma.billing_subscriptions.update).toHaveBeenCalledWith({
      where: { id: 10 },
      data: {
        status: 'canceled',
        cancel_at_period_end: false,
        canceled_at: expect.any(Date),
        ended_at: expect.any(Date),
        updated_at: expect.any(Date)
      }
    });

    expect(result.status).toBe('canceled');
    expect(result.canceled_at).toBeTruthy();
    expect(result.ended_at).toBeTruthy();
  });

  it('rejects if subscription does not belong to app', async () => {
    billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(null);

    await expect(
      service.cancelSubscriptionEx('trashtech', 999, { atPeriodEnd: true })
    ).rejects.toThrow('Subscription 999 not found for app trashtech');
  });

  it('continues if Tilled update fails for cancel_at_period_end (warn only)', async () => {
    const mockSubscription = {
      id: 10,
      tilled_subscription_id: 'sub_test',
      billing_customers: { app_id: 'trashtech' }
    };

    billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockSubscription);
    mockTilledClient.updateSubscription.mockRejectedValue(new Error('Tilled API error'));
    billingPrisma.billing_subscriptions.update.mockResolvedValue({
      ...mockSubscription,
      cancel_at_period_end: true
    });

    const result = await service.cancelSubscriptionEx('trashtech', 10, { atPeriodEnd: true });

    // Should still complete local update
    expect(result.cancel_at_period_end).toBe(true);
  });

  it('defaults to immediate cancel when atPeriodEnd not specified', async () => {
    const mockSubscription = {
      id: 10,
      tilled_subscription_id: 'sub_test',
      billing_customers: { app_id: 'trashtech' }
    };

    billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockSubscription);
    mockTilledClient.cancelSubscription.mockResolvedValue({
      status: 'canceled',
      canceled_at: Math.floor(Date.now() / 1000)
    });
    billingPrisma.billing_subscriptions.update.mockResolvedValue({});

    await service.cancelSubscriptionEx('trashtech', 10, {});

    expect(mockTilledClient.cancelSubscription).toHaveBeenCalled();
  });
});

describe('BillingService.changeCycle', () => {
  let service;
  let mockTilledClient;

  beforeEach(() => {
    service = new BillingService();
    mockTilledClient = {
      createSubscription: jest.fn(),
      cancelSubscription: jest.fn(),
      attachPaymentMethod: jest.fn()
    };
    TilledClient.mockImplementation(() => mockTilledClient);
    jest.clearAllMocks();
  });

  it('cancels old and creates new subscription, returns both', async () => {
    const payload = {
      billing_customer_id: 1,
      from_subscription_id: 10,
      new_plan_id: 'pro-annual',
      new_plan_name: 'Pro Annual',
      price_cents: 99900,
      payment_method_id: 'pm_test',
      payment_method_type: 'card',
      options: {
        metadata: { upgrade: true }
      }
    };

    const mockCustomer = {
      id: 1,
      app_id: 'trashtech',
      tilled_customer_id: 'cus_test'
    };

    const mockOldSubscription = {
      id: 10,
      billing_customer_id: 1,
      tilled_subscription_id: 'sub_old',
      status: 'active',
      billing_customers: mockCustomer
    };

    const mockTilledNewSub = {
      id: 'sub_new',
      status: 'active',
      current_period_start: Math.floor(Date.now() / 1000),
      current_period_end: Math.floor(Date.now() / 1000) + 31536000  // +1 year
    };

    const mockCanceledOldSub = {
      ...mockOldSubscription,
      status: 'canceled',
      canceled_at: new Date(),
      ended_at: new Date()
    };

    const mockNewSubscription = {
      id: 11,
      billing_customer_id: 1,
      tilled_subscription_id: 'sub_new',
      plan_id: 'pro-annual',
      plan_name: 'Pro Annual',
      price_cents: 99900,
      status: 'active'
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockOldSubscription);

    // Mock transaction to execute both updates
    billingPrisma.$transaction.mockImplementation(async (callback) => {
      return callback({
        billing_subscriptions: {
          update: jest.fn().mockResolvedValue(mockCanceledOldSub),
          create: jest.fn().mockResolvedValue(mockNewSubscription)
        }
      });
    });

    mockTilledClient.attachPaymentMethod.mockResolvedValue({ id: 'pm_test', type: 'card' });
    mockTilledClient.createSubscription.mockResolvedValue(mockTilledNewSub);
    mockTilledClient.cancelSubscription.mockResolvedValue({
      status: 'canceled',
      canceled_at: Math.floor(Date.now() / 1000)
    });

    const result = await service.changeCycle('trashtech', payload);

    expect(mockTilledClient.createSubscription).toHaveBeenCalled();
    expect(mockTilledClient.cancelSubscription).toHaveBeenCalledWith('sub_old');

    expect(result.canceled_subscription).toBeTruthy();
    expect(result.new_subscription).toBeTruthy();
    expect(result.new_subscription.plan_id).toBe('pro-annual');
    expect(result.canceled_subscription.status).toBe('canceled');
  });

  it('404 when from_subscription_id not in app scope', async () => {
    const payload = {
      billing_customer_id: 1,
      from_subscription_id: 999,
      new_plan_id: 'pro-annual',
      new_plan_name: 'Pro Annual',
      price_cents: 99900,
      payment_method_id: 'pm_test'
    };

    const mockCustomer = {
      id: 1,
      app_id: 'trashtech'
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(null);

    await expect(
      service.changeCycle('trashtech', payload)
    ).rejects.toThrow('Subscription 999 not found for app trashtech');
  });

  it('400 when required fields missing', async () => {
    const invalidPayload = {
      billing_customer_id: 1,
      // Missing from_subscription_id, plan_id, etc.
    };

    await expect(
      service.changeCycle('trashtech', invalidPayload)
    ).rejects.toThrow();
  });

  it('verifies customer belongs to app before processing', async () => {
    const payload = {
      billing_customer_id: 1,
      from_subscription_id: 10,
      new_plan_id: 'pro-annual',
      new_plan_name: 'Pro Annual',
      price_cents: 99900,
      payment_method_id: 'pm_test'
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

    await expect(
      service.changeCycle('trashtech', payload)
    ).rejects.toThrow('Customer 1 not found for app trashtech');
  });

  it('rolls back if new subscription creation fails', async () => {
    const payload = {
      billing_customer_id: 1,
      from_subscription_id: 10,
      new_plan_id: 'pro-annual',
      new_plan_name: 'Pro Annual',
      price_cents: 99900,
      payment_method_id: 'pm_test'
    };

    const mockCustomer = {
      id: 1,
      app_id: 'trashtech',
      tilled_customer_id: 'cus_test'
    };

    const mockOldSubscription = {
      id: 10,
      tilled_subscription_id: 'sub_old',
      billing_customers: mockCustomer
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockOldSubscription);

    mockTilledClient.attachPaymentMethod.mockResolvedValue({ id: 'pm_test' });
    mockTilledClient.createSubscription.mockRejectedValue(new Error('Tilled API error'));

    await expect(
      service.changeCycle('trashtech', payload)
    ).rejects.toThrow('Tilled API error');

    // Transaction should not have been committed
    expect(billingPrisma.$transaction).not.toHaveBeenCalled();
  });

  it('handles interval_unit and interval_count from options', async () => {
    const payload = {
      billing_customer_id: 1,
      from_subscription_id: 10,
      new_plan_id: 'pro-annual',
      new_plan_name: 'Pro Annual',
      price_cents: 99900,
      payment_method_id: 'pm_test',
      options: {
        intervalUnit: 'year',
        intervalCount: 1
      }
    };

    const mockCustomer = {
      id: 1,
      app_id: 'trashtech',
      tilled_customer_id: 'cus_test'
    };

    const mockOldSubscription = {
      id: 10,
      tilled_subscription_id: 'sub_old',
      billing_customers: mockCustomer
    };

    billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockOldSubscription);

    mockTilledClient.attachPaymentMethod.mockResolvedValue({ id: 'pm_test' });
    mockTilledClient.createSubscription.mockResolvedValue({
      id: 'sub_new',
      status: 'active',
      current_period_start: Math.floor(Date.now() / 1000),
      current_period_end: Math.floor(Date.now() / 1000) + 31536000
    });
    mockTilledClient.cancelSubscription.mockResolvedValue({ status: 'canceled' });

    billingPrisma.$transaction.mockImplementation(async (callback) => {
      return callback({
        billing_subscriptions: {
          update: jest.fn().mockResolvedValue({}),
          create: jest.fn().mockResolvedValue({ id: 11 })
        }
      });
    });

    await service.changeCycle('trashtech', payload);

    expect(mockTilledClient.createSubscription).toHaveBeenCalledWith(
      'cus_test',
      'pm_test',
      99900,
      expect.objectContaining({
        intervalUnit: 'year',
        intervalCount: 1
      })
    );
  });
});
