/**
 * Integration tests for BillingService with real database
 *
 * @integration - Uses real database (mocks Tilled API)
 * Dependencies: MySQL database, Prisma client
 * Cleanup: beforeEach truncates all tables
 */

const BillingService = require('../../backend/src/billingService');
const TilledClient = require('../../backend/src/tilledClient');
const { billingPrisma } = require('../../backend/src/prisma');
const {
  TEST_CUSTOMERS,
  TILLED_CUSTOMER_RESPONSE,
  TILLED_PAYMENT_METHOD_RESPONSE,
  TILLED_SUBSCRIPTION_RESPONSE,
  WEBHOOK_EVENTS,
  generateWebhookSignature
} = require('../fixtures/test-fixtures');
const { cleanDatabase, setupIntegrationTests, teardownIntegrationTests } = require('./database-cleanup');

// Mock Tilled SDK
jest.mock('../../backend/src/tilledClient');

describe('BillingService Integration', () => {
  let service;
  let mockTilledClient;

  beforeAll(async () => {
    await setupIntegrationTests();
  });

  beforeEach(async () => {
    await cleanDatabase();

    service = new BillingService();

    mockTilledClient = {
      createCustomer: jest.fn(),
      attachPaymentMethod: jest.fn(),
      createSubscription: jest.fn(),
      cancelSubscription: jest.fn(),
      verifyWebhookSignature: jest.fn()
    };

    TilledClient.mockImplementation(() => mockTilledClient);
  });

  afterAll(async () => {
    await teardownIntegrationTests();
  });

  describe('createCustomer with database', () => {
    it('should persist customer to database', async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);

      const customer = await service.createCustomer(
        TEST_CUSTOMERS.standard.app_id,
        TEST_CUSTOMERS.standard.email,
        TEST_CUSTOMERS.standard.name,
        TEST_CUSTOMERS.standard.external_customer_id,
        TEST_CUSTOMERS.standard.metadata
      );

      // Verify in database
      const dbCustomer = await billingPrisma.billing_customers.findUnique({
        where: { id: customer.id }
      });

      expect(dbCustomer).toBeTruthy();
      expect(dbCustomer.email).toBe(TEST_CUSTOMERS.standard.email);
      expect(dbCustomer.tilled_customer_id).toBe(TILLED_CUSTOMER_RESPONSE.id);
      expect(dbCustomer.app_id).toBe(TEST_CUSTOMERS.standard.app_id);
    });

    it('should enforce unique constraint on app_id + external_customer_id', async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);

      // Create first customer
      await service.createCustomer(
        TEST_CUSTOMERS.standard.app_id,
        TEST_CUSTOMERS.standard.email,
        TEST_CUSTOMERS.standard.name,
        TEST_CUSTOMERS.standard.external_customer_id,
        TEST_CUSTOMERS.standard.metadata
      );

      // Try to create duplicate (same app_id + external_customer_id)
      mockTilledClient.createCustomer.mockResolvedValue({
        ...TILLED_CUSTOMER_RESPONSE,
        id: 'cus_different_123'
      });

      await expect(
        service.createCustomer(
          TEST_CUSTOMERS.standard.app_id,
          'different@email.com',
          'Different Name',
          TEST_CUSTOMERS.standard.external_customer_id, // Same external_customer_id
          {}
        )
      ).rejects.toThrow();
    });

    it('should allow same external_customer_id for different apps', async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);

      // Create for trashtech
      await service.createCustomer(
        'trashtech',
        'trash@example.com',
        'Trash Customer',
        '123', // external_customer_id
        {}
      );

      // Create for apping with same external_customer_id (should succeed)
      mockTilledClient.createCustomer.mockResolvedValue({
        ...TILLED_CUSTOMER_RESPONSE,
        id: 'cus_apping_123'
      });

      const appingCustomer = await service.createCustomer(
        'apping',
        'apping@example.com',
        'Apping Customer',
        '123', // Same external_customer_id, different app_id
        {}
      );

      expect(appingCustomer).toBeTruthy();
      expect(appingCustomer.app_id).toBe('apping');
    });
  });

  describe('createSubscription with database', () => {
    let testCustomer;

    beforeEach(async () => {
      // Create test customer
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      testCustomer = await service.createCustomer(
        TEST_CUSTOMERS.standard.app_id,
        TEST_CUSTOMERS.standard.email,
        TEST_CUSTOMERS.standard.name,
        TEST_CUSTOMERS.standard.external_customer_id,
        TEST_CUSTOMERS.standard.metadata
      );
    });

    it('should persist subscription to database', async () => {
      mockTilledClient.attachPaymentMethod.mockResolvedValue(TILLED_PAYMENT_METHOD_RESPONSE);
      mockTilledClient.createSubscription.mockResolvedValue(TILLED_SUBSCRIPTION_RESPONSE);

      const subscription = await service.createSubscription(
        testCustomer.app_id,
        testCustomer.id,
        'pm_test_123',
        'pro-monthly',
        'Pro Monthly',
        9900,
        { intervalUnit: 'month', intervalCount: 1 }
      );

      // Verify in database
      const dbSubscription = await billingPrisma.billing_subscriptions.findUnique({
        where: { id: subscription.id }
      });

      expect(dbSubscription).toBeTruthy();
      expect(dbSubscription.plan_id).toBe('pro-monthly');
      expect(dbSubscription.price_cents).toBe(9900);
      expect(dbSubscription.status).toBe('active');
      expect(dbSubscription.billing_customer_id).toBe(testCustomer.id);
    });

    it('should enforce unique constraint on tilled_subscription_id', async () => {
      mockTilledClient.attachPaymentMethod.mockResolvedValue(TILLED_PAYMENT_METHOD_RESPONSE);
      mockTilledClient.createSubscription.mockResolvedValue(TILLED_SUBSCRIPTION_RESPONSE);

      // Create first subscription
      await service.createSubscription(
        testCustomer.app_id,
        testCustomer.id,
        'pm_test_123',
        'pro-monthly',
        'Pro Monthly',
        9900
      );

      // Try to create duplicate with same tilled_subscription_id
      mockTilledClient.attachPaymentMethod.mockResolvedValue(TILLED_PAYMENT_METHOD_RESPONSE);
      mockTilledClient.createSubscription.mockResolvedValue(TILLED_SUBSCRIPTION_RESPONSE); // Same ID

      await expect(
        service.createSubscription(
          testCustomer.app_id,
          testCustomer.id,
          'pm_test_456',
          'pro-annual',
          'Pro Annual',
          99000
        )
      ).rejects.toThrow();
    });

    it('should link subscription to customer via foreign key', async () => {
      mockTilledClient.attachPaymentMethod.mockResolvedValue(TILLED_PAYMENT_METHOD_RESPONSE);
      mockTilledClient.createSubscription.mockResolvedValue(TILLED_SUBSCRIPTION_RESPONSE);

      const subscription = await service.createSubscription(
        testCustomer.app_id,
        testCustomer.id,
        'pm_test_123',
        'pro-monthly',
        'Pro Monthly',
        9900
      );

      // Query with relation
      const customerWithSubs = await billingPrisma.billing_customers.findUnique({
        where: { id: testCustomer.id },
        include: { billing_subscriptions: true }
      });

      expect(customerWithSubs.billing_subscriptions).toHaveLength(1);
      expect(customerWithSubs.billing_subscriptions[0].id).toBe(subscription.id);
    });
  });

  describe('webhook idempotency with database', () => {
    it('should prevent duplicate webhook processing via unique constraint', async () => {
      const event = WEBHOOK_EVENTS.subscriptionCreated;
      const rawBody = JSON.stringify(event);
      const signature = generateWebhookSignature(event, 'whsec_123');

      mockTilledClient.verifyWebhookSignature.mockReturnValue(true);

      // First webhook - should process
      const result1 = await service.processWebhook('trashtech', event, rawBody, signature);
      expect(result1.success).toBe(true);
      expect(result1.duplicate).toBe(false);

      // Verify webhook record created
      const webhook1 = await billingPrisma.billing_webhooks.findFirst({
        where: { event_id: event.id, app_id: 'trashtech' }
      });
      expect(webhook1).toBeTruthy();
      expect(webhook1.status).toBe('processed');

      // Second webhook with same event_id - should detect duplicate
      const result2 = await service.processWebhook('trashtech', event, rawBody, signature);
      expect(result2.success).toBe(true);
      expect(result2.duplicate).toBe(true);

      // Verify only one webhook record exists
      const webhookCount = await billingPrisma.billing_webhooks.count({
        where: { event_id: event.id }
      });
      expect(webhookCount).toBe(1);
    });

    it('should track webhook processing status in database', async () => {
      const event = WEBHOOK_EVENTS.subscriptionCreated;
      const rawBody = JSON.stringify(event);
      const signature = generateWebhookSignature(event, 'whsec_123');

      mockTilledClient.verifyWebhookSignature.mockReturnValue(true);

      await service.processWebhook('trashtech', event, rawBody, signature);

      const webhook = await billingPrisma.billing_webhooks.findFirst({
        where: { event_id: event.id, app_id: 'trashtech' }
      });

      expect(webhook.status).toBe('processed');
      expect(webhook.event_type).toBe('subscription.created');
      expect(webhook.app_id).toBe('trashtech');
      expect(webhook.processed_at).toBeTruthy();
      expect(webhook.error).toBeNull();
    });

    it('should store error details when webhook processing fails', async () => {
      const event = WEBHOOK_EVENTS.subscriptionUpdated;
      const rawBody = JSON.stringify(event);
      const signature = 'invalid-signature';

      mockTilledClient.verifyWebhookSignature.mockReturnValue(false);

      const result = await service.processWebhook('trashtech', event, rawBody, signature);

      expect(result.success).toBe(false);

      const webhook = await billingPrisma.billing_webhooks.findFirst({
        where: { event_id: event.id, app_id: 'trashtech' }
      });

      expect(webhook.status).toBe('failed');
      expect(webhook.error).toBe('Invalid signature');
      expect(webhook.processed_at).toBeTruthy();
    });
  });

  describe('multi-app isolation', () => {
    it('should isolate customers by app_id', async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);

      // Create trashtech customer
      await service.createCustomer('trashtech', 'trash@test.com', 'Trash Co', '1', {});

      // Create apping customer
      mockTilledClient.createCustomer.mockResolvedValue({
        ...TILLED_CUSTOMER_RESPONSE,
        id: 'cus_apping_123'
      });
      await service.createCustomer('apping', 'apping@test.com', 'Apping Co', '1', {});

      // Query by app_id
      const trashCustomers = await billingPrisma.billing_customers.findMany({
        where: { app_id: 'trashtech' }
      });
      const appingCustomers = await billingPrisma.billing_customers.findMany({
        where: { app_id: 'apping' }
      });

      expect(trashCustomers).toHaveLength(1);
      expect(appingCustomers).toHaveLength(1);
      expect(trashCustomers[0].email).toBe('trash@test.com');
      expect(appingCustomers[0].email).toBe('apping@test.com');
    });
  });
});
