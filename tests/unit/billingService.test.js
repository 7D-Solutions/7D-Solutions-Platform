const BillingService = require('../../backend/src/billingService');
const TilledClient = require('../../backend/src/tilledClient');
const { billingPrisma } = require('../../backend/src/prisma');
const {
  TEST_CUSTOMERS,
  TILLED_CUSTOMER_RESPONSE,
  TILLED_PAYMENT_METHOD_RESPONSE,
  TILLED_SUBSCRIPTION_RESPONSE,
  TEST_SUBSCRIPTIONS,
  WEBHOOK_EVENTS
} = require('../fixtures/test-fixtures');
const { cleanDatabase, createTestCustomer, createTestSubscription } = require('../helpers');

// Mock dependencies
jest.mock('../../backend/src/tilledClient');
jest.mock('../../backend/src/prisma', () => ({
    billingPrisma: {
      billing_customers: {
        create: jest.fn(),
        findUnique: jest.fn(),
        findFirst: jest.fn(),
        update: jest.fn()
      },
    billing_subscriptions: {
      create: jest.fn(),
      findUnique: jest.fn(),
      findFirst: jest.fn(),
      update: jest.fn()
    },
    billing_webhooks: {
      create: jest.fn(),
      update: jest.fn()
    }
  }
}));

describe('BillingService', () => {
  let service;
  let mockTilledClient;

  beforeEach(() => {
    service = new BillingService();

    // Create mock Tilled client
    mockTilledClient = {
      createCustomer: jest.fn(),
      attachPaymentMethod: jest.fn(),
      createSubscription: jest.fn(),
      cancelSubscription: jest.fn(),
      verifyWebhookSignature: jest.fn()
    };

    TilledClient.mockImplementation(() => mockTilledClient);

    // Reset all mocks
    jest.clearAllMocks();
  });

  describe('createCustomer', () => {
    it('should create customer in Tilled and database', async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      billingPrisma.billing_customers.create.mockResolvedValue({
        id: 1,
        ...TEST_CUSTOMERS.standard,
        tilled_customer_id: TILLED_CUSTOMER_RESPONSE.id
      });

      const result = await service.createCustomer(
        TEST_CUSTOMERS.standard.app_id,
        TEST_CUSTOMERS.standard.email,
        TEST_CUSTOMERS.standard.name,
        TEST_CUSTOMERS.standard.external_customer_id,
        TEST_CUSTOMERS.standard.metadata
      );

      expect(mockTilledClient.createCustomer).toHaveBeenCalledWith(
        TEST_CUSTOMERS.standard.email,
        TEST_CUSTOMERS.standard.name,
        TEST_CUSTOMERS.standard.metadata
      );

      expect(billingPrisma.billing_customers.create).toHaveBeenCalledWith({
        data: {
          app_id: TEST_CUSTOMERS.standard.app_id,
          external_customer_id: TEST_CUSTOMERS.standard.external_customer_id,
          tilled_customer_id: TILLED_CUSTOMER_RESPONSE.id,
          email: TEST_CUSTOMERS.standard.email,
          name: TEST_CUSTOMERS.standard.name,
          metadata: TEST_CUSTOMERS.standard.metadata
        }
      });

      expect(result.id).toBe(1);
      expect(result.tilled_customer_id).toBe(TILLED_CUSTOMER_RESPONSE.id);
    });

    it('should handle customer without external_customer_id', async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      billingPrisma.billing_customers.create.mockResolvedValue({
        id: 2,
        ...TEST_CUSTOMERS.noExternal,
        tilled_customer_id: TILLED_CUSTOMER_RESPONSE.id
      });

      const result = await service.createCustomer(
        TEST_CUSTOMERS.noExternal.app_id,
        TEST_CUSTOMERS.noExternal.email,
        TEST_CUSTOMERS.noExternal.name,
        null,
        {}
      );

      expect(billingPrisma.billing_customers.create).toHaveBeenCalledWith({
        data: expect.objectContaining({
          external_customer_id: null
        })
      });
    });
  });

  describe('setDefaultPaymentMethod', () => {
    it('should update customer with default payment method', async () => {
      const appId = TEST_CUSTOMERS.standard.app_id;
      const customerId = 1;
      const paymentMethodId = 'pm_test_123';
      const paymentMethodType = 'card';

      billingPrisma.billing_customers.findFirst.mockResolvedValue({
        id: customerId,
        app_id: appId
      });
      billingPrisma.billing_customers.update.mockResolvedValue({
        id: customerId,
        default_payment_method_id: paymentMethodId,
        payment_method_type: paymentMethodType
      });

      const result = await service.setDefaultPaymentMethod(
        appId,
        customerId,
        paymentMethodId,
        paymentMethodType
      );

      expect(billingPrisma.billing_customers.update).toHaveBeenCalledWith({
        where: { id: customerId },
        data: {
          default_payment_method_id: paymentMethodId,
          payment_method_type: paymentMethodType,
          updated_at: expect.any(Date)
        }
      });

      expect(result.default_payment_method_id).toBe(paymentMethodId);
    });
  });

  describe('createSubscription', () => {
    it('should create subscription with full flow', async () => {
      const testCustomer = {
        id: 1,
        app_id: 'trashtech',
        tilled_customer_id: 'cus_test_123'
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(testCustomer);
      mockTilledClient.attachPaymentMethod.mockResolvedValue(TILLED_PAYMENT_METHOD_RESPONSE);
      mockTilledClient.createSubscription.mockResolvedValue(TILLED_SUBSCRIPTION_RESPONSE);
      billingPrisma.billing_subscriptions.create.mockResolvedValue({
        id: 1,
        ...TEST_SUBSCRIPTIONS.monthly
      });

      const result = await service.createSubscription(
        testCustomer.app_id,
        TEST_SUBSCRIPTIONS.monthly.billing_customer_id,
        TEST_SUBSCRIPTIONS.monthly.payment_method_id,
        TEST_SUBSCRIPTIONS.monthly.plan_id,
        TEST_SUBSCRIPTIONS.monthly.plan_name,
        TEST_SUBSCRIPTIONS.monthly.price_cents,
        TEST_SUBSCRIPTIONS.monthly.options
      );

      // Verify payment method attached
      expect(mockTilledClient.attachPaymentMethod).toHaveBeenCalledWith(
        TEST_SUBSCRIPTIONS.monthly.payment_method_id,
        testCustomer.tilled_customer_id
      );

      // Verify subscription created in Tilled
      expect(mockTilledClient.createSubscription).toHaveBeenCalledWith(
        testCustomer.tilled_customer_id,
        TEST_SUBSCRIPTIONS.monthly.payment_method_id,
        TEST_SUBSCRIPTIONS.monthly.price_cents,
        TEST_SUBSCRIPTIONS.monthly.options
      );

      // Verify subscription saved to database
      expect(billingPrisma.billing_subscriptions.create).toHaveBeenCalled();
      expect(result.id).toBe(1);
    });

    it('should throw error if customer not found', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(
        service.createSubscription(TEST_CUSTOMERS.standard.app_id, 999, 'pm_test', 'plan_id', 'Plan Name', 9900)
      ).rejects.toThrow(`Billing customer 999 not found for app ${TEST_CUSTOMERS.standard.app_id}`);
    });

    it('should handle ACH payment method', async () => {
      const testCustomer = {
        id: 1,
        app_id: 'trashtech',
        tilled_customer_id: 'cus_test_123'
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(testCustomer);
      mockTilledClient.attachPaymentMethod.mockResolvedValue({
        id: 'pm_ach_test',
        type: 'ach_debit'
      });
      mockTilledClient.createSubscription.mockResolvedValue(TILLED_SUBSCRIPTION_RESPONSE);
      billingPrisma.billing_subscriptions.create.mockResolvedValue({
        id: 1,
        payment_method_type: 'ach_debit'
      });

      await service.createSubscription(
        testCustomer.app_id,
        1,
        'pm_ach_test',
        'plan_id',
        'Plan Name',
        9900
      );

      expect(billingPrisma.billing_subscriptions.create).toHaveBeenCalledWith({
        data: expect.objectContaining({
          payment_method_type: 'ach_debit'
        })
      });
    });
  });

  describe('cancelSubscription', () => {
    it('should cancel subscription in Tilled and update database', async () => {
      const testSubscription = {
        id: 1,
        billing_customer_id: 1,
        tilled_subscription_id: 'sub_test_123',
        status: 'active'
      };

      const testCustomer = {
        id: 1,
        app_id: 'trashtech'
      };

      billingPrisma.billing_subscriptions.findUnique.mockResolvedValue(testSubscription);
      billingPrisma.billing_customers.findUnique.mockResolvedValue(testCustomer);
      mockTilledClient.cancelSubscription.mockResolvedValue({
        ...TILLED_SUBSCRIPTION_RESPONSE,
        status: 'canceled',
        canceled_at: Math.floor(Date.now() / 1000)
      });
      billingPrisma.billing_subscriptions.update.mockResolvedValue({
        ...testSubscription,
        status: 'canceled'
      });

      const result = await service.cancelSubscription(1);

      expect(mockTilledClient.cancelSubscription).toHaveBeenCalledWith('sub_test_123');
      expect(billingPrisma.billing_subscriptions.update).toHaveBeenCalledWith({
        where: { id: 1 },
        data: {
          status: 'canceled',
          canceled_at: expect.any(Date),
          updated_at: expect.any(Date)
        }
      });

      expect(result.status).toBe('canceled');
    });

    it('should throw error if subscription not found', async () => {
      billingPrisma.billing_subscriptions.findUnique.mockResolvedValue(null);

      await expect(service.cancelSubscription(999)).rejects.toThrow('Subscription 999 not found');
    });
  });

  describe('processWebhook', () => {
    const rawBody = JSON.stringify(WEBHOOK_EVENTS.subscriptionCreated);
    const signature = 't=1234567890,v1=abc123';

    it('should process webhook with insert-first idempotency', async () => {
      billingPrisma.billing_webhooks.create.mockResolvedValue({
        id: 1,
        event_id: WEBHOOK_EVENTS.subscriptionCreated.id,
        status: 'received'
      });
      mockTilledClient.verifyWebhookSignature.mockReturnValue(true);
      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue({
        id: 1,
        tilled_subscription_id: TILLED_SUBSCRIPTION_RESPONSE.id
      });
      billingPrisma.billing_subscriptions.update.mockResolvedValue({});
      billingPrisma.billing_webhooks.update.mockResolvedValue({});

      const result = await service.processWebhook(
        'trashtech',
        WEBHOOK_EVENTS.subscriptionCreated,
        rawBody,
        signature
      );

      expect(result.success).toBe(true);
      expect(result.duplicate).toBe(false);
      expect(billingPrisma.billing_webhooks.create).toHaveBeenCalledWith({
        data: {
          app_id: 'trashtech',
          event_id: WEBHOOK_EVENTS.subscriptionCreated.id,
          event_type: WEBHOOK_EVENTS.subscriptionCreated.type,
          status: 'received'
        }
      });
    });

    it('should detect duplicate webhook via unique constraint', async () => {
      const duplicateError = new Error('Unique constraint failed');
      duplicateError.code = 'P2002';
      billingPrisma.billing_webhooks.create.mockRejectedValue(duplicateError);

      const result = await service.processWebhook(
        'trashtech',
        WEBHOOK_EVENTS.subscriptionCreated,
        rawBody,
        signature
      );

      expect(result.success).toBe(true);
      expect(result.duplicate).toBe(true);
      expect(mockTilledClient.verifyWebhookSignature).not.toHaveBeenCalled();
    });

    it('should reject webhook with invalid signature', async () => {
      billingPrisma.billing_webhooks.create.mockResolvedValue({
        id: 1,
        event_id: WEBHOOK_EVENTS.subscriptionCreated.id
      });
      mockTilledClient.verifyWebhookSignature.mockReturnValue(false);
      billingPrisma.billing_webhooks.update.mockResolvedValue({});

      const result = await service.processWebhook(
        'trashtech',
        WEBHOOK_EVENTS.subscriptionCreated,
        rawBody,
        'invalid-signature'
      );

      expect(result.success).toBe(false);
      expect(result.error).toBe('Invalid signature');
      expect(billingPrisma.billing_webhooks.update).toHaveBeenCalledWith({
        where: {
          event_id_app_id: {
            event_id: WEBHOOK_EVENTS.subscriptionCreated.id,
            app_id: 'trashtech'
          }
        },
        data: {
          status: 'failed',
          error: 'Invalid signature',
          processed_at: expect.any(Date)
        }
      });
    });

    it('should handle subscription.updated event', async () => {
      billingPrisma.billing_webhooks.create.mockResolvedValue({});
      mockTilledClient.verifyWebhookSignature.mockReturnValue(true);
      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue({
        id: 1,
        tilled_subscription_id: TILLED_SUBSCRIPTION_RESPONSE.id
      });
      billingPrisma.billing_subscriptions.update.mockResolvedValue({});
      billingPrisma.billing_webhooks.update.mockResolvedValue({});

      await service.processWebhook(
        'trashtech',
        WEBHOOK_EVENTS.subscriptionUpdated,
        JSON.stringify(WEBHOOK_EVENTS.subscriptionUpdated),
        signature
      );

      expect(billingPrisma.billing_subscriptions.update).toHaveBeenCalledWith({
        where: { id: 1 },
        data: expect.objectContaining({
          status: 'past_due'
        })
      });
    });
  });

  describe('getCustomerById', () => {
    it('should return customer when found and app_id matches', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech',
        email: 'test@example.com',
        name: 'Test Customer'
      };

      billingPrisma.billing_customers.findFirst = jest.fn().mockResolvedValue(mockCustomer);

      const result = await service.getCustomerById('trashtech', 1);

      expect(billingPrisma.billing_customers.findFirst).toHaveBeenCalledWith({
        where: { id: 1, app_id: 'trashtech' }
      });
      expect(result).toEqual(mockCustomer);
    });

    it('should throw error when customer not found', async () => {
      billingPrisma.billing_customers.findFirst = jest.fn().mockResolvedValue(null);

      await expect(service.getCustomerById('trashtech', 999)).rejects.toThrow(
        'Customer 999 not found for app trashtech'
      );
    });

    it('should throw error when app_id does not match', async () => {
      billingPrisma.billing_customers.findFirst = jest.fn().mockResolvedValue(null);

      await expect(service.getCustomerById('apping', 1)).rejects.toThrow(
        'Customer 1 not found for app apping'
      );
    });
  });

  describe('findCustomer', () => {
    it('should find customer by external_customer_id', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech',
        external_customer_id: '123',
        email: 'test@example.com'
      };

      billingPrisma.billing_customers.findFirst = jest.fn().mockResolvedValue(mockCustomer);

      const result = await service.findCustomer('trashtech', 123);

      expect(billingPrisma.billing_customers.findFirst).toHaveBeenCalledWith({
        where: { app_id: 'trashtech', external_customer_id: '123' }
      });
      expect(result).toEqual(mockCustomer);
    });

    it('should throw error when customer not found', async () => {
      billingPrisma.billing_customers.findFirst = jest.fn().mockResolvedValue(null);

      await expect(service.findCustomer('trashtech', 999)).rejects.toThrow(
        'Customer with external_customer_id 999 not found for app trashtech'
      );
    });
  });

  describe('getSubscriptionById', () => {
    it('should return subscription when found and app_id matches', async () => {
      const mockSubscription = {
        id: 1,
        billing_customer_id: 1,
        status: 'active',
        billing_customers: { app_id: 'trashtech' }
      };

      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockSubscription);

      const result = await service.getSubscriptionById('trashtech', 1);

      expect(billingPrisma.billing_subscriptions.findFirst).toHaveBeenCalledWith({
        where: { id: 1 },
        include: { billing_customers: true }
      });
      expect(result).toEqual(mockSubscription);
    });

    it('should throw error when subscription not found', async () => {
      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(null);

      await expect(service.getSubscriptionById('trashtech', 999)).rejects.toThrow(
        'Subscription 999 not found'
      );
    });

    it('should throw error when app_id does not match', async () => {
      const mockSubscription = {
        id: 1,
        billing_customers: { app_id: 'different_app' }
      };

      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockSubscription);

      await expect(service.getSubscriptionById('trashtech', 1)).rejects.toThrow(
        'Subscription 1 not found for app trashtech'
      );
    });
  });

  describe('listSubscriptions', () => {
    it('should list subscriptions with app_id filter', async () => {
      billingPrisma.billing_subscriptions.findMany = jest.fn().mockResolvedValue([]);

      await service.listSubscriptions({ appId: 'trashtech' });

      expect(billingPrisma.billing_subscriptions.findMany).toHaveBeenCalledWith({
        where: { billing_customers: { app_id: 'trashtech' } },
        include: { billing_customers: true },
        orderBy: { created_at: 'desc' }
      });
    });

    it('should list subscriptions with billing_customer_id filter', async () => {
      billingPrisma.billing_subscriptions.findMany = jest.fn().mockResolvedValue([]);

      await service.listSubscriptions({ billingCustomerId: 1 });

      expect(billingPrisma.billing_subscriptions.findMany).toHaveBeenCalledWith({
        where: { billing_customer_id: 1 },
        include: { billing_customers: true },
        orderBy: { created_at: 'desc' }
      });
    });

    it('should list subscriptions with status filter', async () => {
      billingPrisma.billing_subscriptions.findMany = jest.fn().mockResolvedValue([]);

      await service.listSubscriptions({ status: 'active' });

      expect(billingPrisma.billing_subscriptions.findMany).toHaveBeenCalledWith({
        where: { status: 'active' },
        include: { billing_customers: true },
        orderBy: { created_at: 'desc' }
      });
    });

    it('should combine multiple filters', async () => {
      billingPrisma.billing_subscriptions.findMany = jest.fn().mockResolvedValue([]);

      await service.listSubscriptions({
        appId: 'trashtech',
        status: 'active',
        billingCustomerId: 1
      });

      expect(billingPrisma.billing_subscriptions.findMany).toHaveBeenCalledWith({
        where: {
          billing_customer_id: 1,
          status: 'active',
          billing_customers: { app_id: 'trashtech' }
        },
        include: { billing_customers: true },
        orderBy: { created_at: 'desc' }
      });
    });
  });

  describe('updateCustomer', () => {
    beforeEach(() => {
      billingPrisma.billing_customers.findFirst = jest.fn();
      billingPrisma.billing_customers.update = jest.fn();
      mockTilledClient.updateCustomer = jest.fn();
    });

    it('should update customer and sync to Tilled', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech',
        tilled_customer_id: 'cus_test_123',
        email: 'old@example.com',
        name: 'Old Name'
      };

      const updatedCustomer = {
        ...mockCustomer,
        email: 'new@example.com',
        name: 'New Name'
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_customers.update.mockResolvedValue(updatedCustomer);
      mockTilledClient.updateCustomer.mockResolvedValue({});

      const result = await service.updateCustomer('trashtech', 1, {
        email: 'new@example.com',
        name: 'New Name'
      });

      expect(billingPrisma.billing_customers.update).toHaveBeenCalledWith({
        where: { id: 1 },
        data: {
          email: 'new@example.com',
          name: 'New Name',
          updated_at: expect.any(Date)
        }
      });

      expect(mockTilledClient.updateCustomer).toHaveBeenCalledWith('cus_test_123', {
        email: 'new@example.com',
        name: 'New Name'
      });

      expect(result).toEqual(updatedCustomer);
    });

    it('should throw error when no valid fields provided', async () => {
      const mockCustomer = { id: 1, app_id: 'trashtech' };
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);

      await expect(service.updateCustomer('trashtech', 1, {})).rejects.toThrow(
        'No valid fields to update'
      );
    });

    it('should throw error when customer not found', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(
        service.updateCustomer('trashtech', 999, { email: 'test@example.com' })
      ).rejects.toThrow('Customer 999 not found for app trashtech');
    });
  });

  describe('updateSubscription', () => {
    beforeEach(() => {
      billingPrisma.billing_subscriptions.findFirst = jest.fn();
      billingPrisma.billing_subscriptions.update = jest.fn();
      mockTilledClient.updateSubscription = jest.fn();
    });

    it('should update subscription metadata', async () => {
      const mockSubscription = {
        id: 1,
        tilled_subscription_id: 'sub_test_123',
        billing_customers: { app_id: 'trashtech' }
      };

      const updatedSubscription = {
        ...mockSubscription,
        metadata: { feature: 'premium' }
      };

      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockSubscription);
      billingPrisma.billing_subscriptions.update.mockResolvedValue(updatedSubscription);
      mockTilledClient.updateSubscription.mockResolvedValue({});

      const result = await service.updateSubscription('trashtech', 1, {
        metadata: { feature: 'premium' }
      });

      expect(billingPrisma.billing_subscriptions.update).toHaveBeenCalledWith({
        where: { id: 1 },
        data: {
          metadata: { feature: 'premium' },
          updated_at: expect.any(Date)
        }
      });

      expect(result).toEqual(updatedSubscription);
    });

    it('should reject billing cycle changes', async () => {
      const mockSubscription = {
        id: 1,
        billing_customers: { app_id: 'trashtech' }
      };

      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockSubscription);

      await expect(
        service.updateSubscription('trashtech', 1, { interval_unit: 'year' })
      ).rejects.toThrow('Cannot change billing cycle after subscription creation');
    });

    it('should reject interval_count changes', async () => {
      const mockSubscription = {
        id: 1,
        billing_customers: { app_id: 'trashtech' }
      };

      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockSubscription);

      await expect(
        service.updateSubscription('trashtech', 1, { interval_count: 2 })
      ).rejects.toThrow('Cannot change billing cycle after subscription creation');
    });

    it('should reject billing_cycle_anchor changes', async () => {
      const mockSubscription = {
        id: 1,
        billing_customers: { app_id: 'trashtech' }
      };

      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockSubscription);

      await expect(
        service.updateSubscription('trashtech', 1, { billing_cycle_anchor: 1234567890 })
      ).rejects.toThrow('Cannot change billing cycle after subscription creation');
    });

    it('should throw error when subscription not found', async () => {
      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(null);

      await expect(
        service.updateSubscription('trashtech', 999, { metadata: {} })
      ).rejects.toThrow('Subscription 999 not found');
    });
  });
});
