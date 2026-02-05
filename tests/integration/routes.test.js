/**
 * Integration tests for billing routes
 *
 * @integration - Uses real database, mocked Tilled API
 * Tests full request/response cycle through Express routes
 */

// CRITICAL: Mock must be defined BEFORE any imports that use it
jest.mock('../../backend/src/tilledClient');

const express = require('express');
const request = require('supertest');
const TilledClient = require('../../backend/src/tilledClient');
const { billingPrisma } = require('../../backend/src/prisma');
const { captureRawBody, rejectSensitiveData } = require('../../backend/src/middleware');
const {
  TILLED_CUSTOMER_RESPONSE,
  TILLED_PAYMENT_METHOD_RESPONSE,
  TILLED_SUBSCRIPTION_RESPONSE,
  WEBHOOK_EVENTS,
  generateWebhookSignature
} = require('../fixtures/test-fixtures');
const { cleanDatabase, setupIntegrationTests, teardownIntegrationTests } = require('./database-cleanup');

// Mock Tilled client methods
const mockTilledClient = {
  createCustomer: jest.fn(),
  attachPaymentMethod: jest.fn(),
  createSubscription: jest.fn(),
  cancelSubscription: jest.fn(),
  verifyWebhookSignature: jest.fn(),
  createCharge: jest.fn()
};

TilledClient.mockImplementation(() => mockTilledClient);

describe('Billing Routes Integration', () => {
  let app;

  beforeAll(async () => {
    await setupIntegrationTests();

    // Import routes AFTER mock is set up
    const billingRoutes = require('../../backend/src/routes/index');
    const handleBillingError = require('../../backend/src/middleware/errorHandler');

    // Setup Express app
    app = express();

    // For webhook routes: capture raw body before parsing (test version)
    app.use('/api/billing/webhooks', express.json(), (req, res, next) => {
      // In tests, reconstruct rawBody from parsed body since supertest doesn't trigger stream events
      req.rawBody = JSON.stringify(req.body);
      next();
    }, billingRoutes);

    app.use('/api/billing', express.json(), rejectSensitiveData, billingRoutes);
    app.use(handleBillingError); // Error handler MUST be mounted last
  });

  beforeEach(async () => {
    await cleanDatabase();

    // Reset mock implementations
    jest.clearAllMocks();
  });

  afterAll(async () => {
    await teardownIntegrationTests();
  });

  describe('GET /api/billing/health', () => {
    it('should return health status with all checks', async () => {
      const response = await request(app)
        .get('/api/billing/health?app_id=trashtech')
        .expect(200);

      expect(response.body).toHaveProperty('timestamp');
      expect(response.body).toHaveProperty('app_id', 'trashtech');
      expect(response.body).toHaveProperty('database');
      expect(response.body).toHaveProperty('tilled_config');
      expect(response.body).toHaveProperty('overall_status');

      expect(response.body.database.status).toBe('healthy');
      expect(response.body.tilled_config.status).toBe('healthy');
      expect(response.body.overall_status).toBe('healthy');
    });

    it('should return 400 when app_id missing', async () => {
      await request(app)
        .get('/api/billing/health')
        .expect(400);
    });
  });

  describe('POST /api/billing/customers', () => {
    it('should create customer successfully', async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);

      const response = await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech',
          email: 'test@acmewaste.com',
          name: 'Acme Waste Inc',
          external_customer_id: '1',
          metadata: { industry: 'waste' }
        })
        .expect(201);

      expect(response.body).toMatchObject({
        app_id: 'trashtech',
        email: 'test@acmewaste.com',
        name: 'Acme Waste Inc',
        tilled_customer_id: TILLED_CUSTOMER_RESPONSE.id
      });

      // Verify in database
      const customer = await billingPrisma.billing_customers.findUnique({
        where: { id: response.body.id }
      });
      expect(customer).toBeTruthy();
    });

    it('should return 400 for missing required fields', async () => {
      const response = await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech'
          // Missing email, name
        })
        .expect(400);

      expect(response.body.error).toBe('Validation failed');
      expect(response.body.details).toBeDefined();
      expect(response.body.details.length).toBeGreaterThan(0);
    });

    it('should reject sensitive data', async () => {
      const response = await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech',
          email: 'test@example.com',
          name: 'Test',
          card_number: '4242424242424242' // Forbidden
        })
        .expect(400);

      expect(response.body.error).toContain('PCI violation');
    });
  });

  describe('POST /api/billing/customers/:id/default-payment-method', () => {
    let testCustomer;

    beforeEach(async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      const response = await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech',
          email: 'test@example.com',
          name: 'Test Customer',
          external_customer_id: '1'
        });
      testCustomer = response.body;
    });

    it('should set default payment method', async () => {
      const response = await request(app)
        .post(`/api/billing/customers/${testCustomer.id}/default-payment-method?app_id=trashtech`)
        .send({
          payment_method_id: 'pm_test_123',
          payment_method_type: 'card'
        })
        .expect(200);

      expect(response.body.default_payment_method_id).toBe('pm_test_123');
      expect(response.body.payment_method_type).toBe('card');
    });
  });

  describe('POST /api/billing/subscriptions', () => {
    let testCustomer;

    beforeEach(async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      const response = await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech',
          email: 'test@example.com',
          name: 'Test Customer',
          external_customer_id: '1'
        });
      testCustomer = response.body;
    });

    it('should create subscription successfully', async () => {
      mockTilledClient.attachPaymentMethod.mockResolvedValue(TILLED_PAYMENT_METHOD_RESPONSE);
      mockTilledClient.createSubscription.mockResolvedValue(TILLED_SUBSCRIPTION_RESPONSE);

      const response = await request(app)
        .post('/api/billing/subscriptions?app_id=trashtech')
        .send({
          billing_customer_id: testCustomer.id,
          payment_method_id: 'pm_test_123',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 9900,
          interval_unit: 'month',
          interval_count: 1
        })
        .expect(201);

      expect(response.body).toMatchObject({
        plan_id: 'pro-monthly',
        price_cents: 9900,
        status: 'active',
        billing_customer_id: testCustomer.id
      });

      // Verify in database
      const subscription = await billingPrisma.billing_subscriptions.findUnique({
        where: { id: response.body.id }
      });
      expect(subscription).toBeTruthy();
    });

    it('should return 400 for missing required fields', async () => {
      await request(app)
        .post('/api/billing/subscriptions?app_id=trashtech')
        .send({
          billing_customer_id: testCustomer.id
          // Missing other fields
        })
        .expect(400);
    });
  });

  describe('DELETE /api/billing/subscriptions/:id', () => {
    let testSubscription;

    beforeEach(async () => {
      // Create customer
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      const customerResponse = await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech',
          email: 'test@example.com',
          name: 'Test Customer',
          external_customer_id: '1'
        });

      // Create subscription
      mockTilledClient.attachPaymentMethod.mockResolvedValue(TILLED_PAYMENT_METHOD_RESPONSE);
      mockTilledClient.createSubscription.mockResolvedValue(TILLED_SUBSCRIPTION_RESPONSE);
      const subscriptionResponse = await request(app)
        .post('/api/billing/subscriptions?app_id=trashtech')
        .send({
          billing_customer_id: customerResponse.body.id,
          payment_method_id: 'pm_test_123',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 9900
        });

      testSubscription = subscriptionResponse.body;
    });

    it('should cancel subscription successfully', async () => {
      mockTilledClient.cancelSubscription.mockResolvedValue({
        ...TILLED_SUBSCRIPTION_RESPONSE,
        status: 'canceled',
        canceled_at: Math.floor(Date.now() / 1000)
      });

      const response = await request(app)
        .delete(`/api/billing/subscriptions/${testSubscription.id}`)
        .query({ app_id: 'trashtech', at_period_end: 'false' })
        .expect(200);

      expect(response.body.status).toBe('canceled');
      expect(response.body.canceled_at).toBeTruthy();

      // Verify in database
      const subscription = await billingPrisma.billing_subscriptions.findUnique({
        where: { id: testSubscription.id }
      });
      expect(subscription.status).toBe('canceled');
    });

    it('should return 404 for non-existent subscription', async () => {
      await request(app)
        .delete('/api/billing/subscriptions/99999')
        .query({ app_id: 'trashtech' })
        .expect(404);
    });
  });

  describe('POST /api/billing/webhooks/:app_id', () => {
    it('should process valid webhook', async () => {
      const event = WEBHOOK_EVENTS.subscriptionCreated;
      const rawBody = JSON.stringify(event);
      const signature = generateWebhookSignature(event, 'whsec_123');

      mockTilledClient.verifyWebhookSignature.mockReturnValue(true);

      const response = await request(app)
        .post('/api/billing/webhooks/trashtech')
        .set('payments-signature', signature)
        .send(event)
        .expect(200);

      expect(response.body.received).toBe(true);
      expect(response.body.duplicate).toBe(false);

      // Verify webhook stored
      const webhook = await billingPrisma.billing_webhooks.findFirst({
        where: { event_id: event.id, app_id: 'trashtech' }
      });
      expect(webhook).toBeTruthy();
      expect(webhook.status).toBe('processed');
    });

    it('should detect duplicate webhooks', async () => {
      const event = WEBHOOK_EVENTS.subscriptionCreated;
      const signature = generateWebhookSignature(event, 'whsec_123');

      mockTilledClient.verifyWebhookSignature.mockReturnValue(true);

      // First webhook
      await request(app)
        .post('/api/billing/webhooks/trashtech')
        .set('payments-signature', signature)
        .send(event)
        .expect(200);

      // Second webhook (duplicate)
      const response = await request(app)
        .post('/api/billing/webhooks/trashtech')
        .set('payments-signature', signature)
        .send(event)
        .expect(200);

      expect(response.body.duplicate).toBe(true);
    });

    it('should handle concurrent duplicate webhooks (race condition)', async () => {
      const event = {
        id: 'evt_concurrent_test_' + Date.now(),
        type: 'subscription.created',
        data: {
          object: {
            id: 'sub_test_concurrent',
            status: 'active',
            customer: 'cus_test',
            current_period_start: Math.floor(Date.now() / 1000),
            current_period_end: Math.floor(Date.now() / 1000) + 2592000
          }
        }
      };
      const signature = generateWebhookSignature(event, 'whsec_123');

      mockTilledClient.verifyWebhookSignature.mockReturnValue(true);

      // Send same webhook concurrently (simulates race condition)
      const responses = await Promise.all([
        request(app)
          .post('/api/billing/webhooks/trashtech')
          .set('payments-signature', signature)
          .send(event),
        request(app)
          .post('/api/billing/webhooks/trashtech')
          .set('payments-signature', signature)
          .send(event),
        request(app)
          .post('/api/billing/webhooks/trashtech')
          .set('payments-signature', signature)
          .send(event)
      ]);

      // All requests should succeed
      responses.forEach(response => {
        expect(response.status).toBe(200);
        expect(response.body.received).toBe(true);
      });

      // Exactly one should be processed, others should be duplicates
      const duplicateCount = responses.filter(r => r.body.duplicate === true).length;
      const processedCount = responses.filter(r => r.body.duplicate === false).length;

      expect(processedCount).toBe(1);
      expect(duplicateCount).toBe(2);

      // Verify only one webhook record exists in database
      const webhookCount = await billingPrisma.billing_webhooks.count({
        where: { event_id: event.id }
      });
      expect(webhookCount).toBe(1);

      // Verify the webhook was successfully processed
      const webhook = await billingPrisma.billing_webhooks.findFirst({
        where: { event_id: event.id, app_id: 'trashtech' }
      });
      expect(webhook.status).toBe('processed');
    });

    it('should reject webhook with invalid signature', async () => {
      const event = WEBHOOK_EVENTS.subscriptionCreated;
      const signature = 'invalid-signature';

      mockTilledClient.verifyWebhookSignature.mockReturnValue(false);

      await request(app)
        .post('/api/billing/webhooks/trashtech')
        .set('payments-signature', signature)
        .send(event)
        .expect(401);
    });

    it('should reject webhook without signature header', async () => {
      const event = WEBHOOK_EVENTS.subscriptionCreated;

      await request(app)
        .post('/api/billing/webhooks/trashtech')
        .send(event)
        .expect(401);
    });
  });

  describe('GET /api/billing/customers/:id', () => {
    let testCustomer;

    beforeEach(async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      const response = await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech',
          email: 'test@example.com',
          name: 'Test Customer',
          external_customer_id: '1'
        });
      testCustomer = response.body;
    });

    it('should get customer by id', async () => {
      const response = await request(app)
        .get(`/api/billing/customers/${testCustomer.id}?app_id=trashtech`)
        .expect(200);

      expect(response.body.id).toBe(testCustomer.id);
      expect(response.body.email).toBe('test@example.com');
    });

    it('should return 404 for non-existent customer', async () => {
      await request(app)
        .get('/api/billing/customers/99999?app_id=trashtech')
        .expect(404);
    });

    it('should return 404 for wrong app_id', async () => {
      await request(app)
        .get(`/api/billing/customers/${testCustomer.id}?app_id=different_app`)
        .expect(404);
    });

    it('should return 400 when app_id missing', async () => {
      await request(app)
        .get(`/api/billing/customers/${testCustomer.id}`)
        .expect(400);
    });
  });

  describe('GET /api/billing/customers (by external_customer_id)', () => {
    beforeEach(async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech',
          email: 'test@example.com',
          name: 'Test Customer',
          external_customer_id: '123'
        });
    });

    it('should find customer by external_customer_id', async () => {
      const response = await request(app)
        .get('/api/billing/customers?app_id=trashtech&external_customer_id=123')
        .expect(200);

      expect(response.body.external_customer_id).toBe('123');
      expect(response.body.email).toBe('test@example.com');
    });

    it('should return 404 when not found', async () => {
      await request(app)
        .get('/api/billing/customers?app_id=trashtech&external_customer_id=999')
        .expect(404);
    });

    it('should return 400 when parameters missing', async () => {
      await request(app)
        .get('/api/billing/customers?app_id=trashtech')
        .expect(400);
    });
  });

  describe('PUT /api/billing/customers/:id', () => {
    let testCustomer;

    beforeEach(async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      const response = await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech',
          email: 'old@example.com',
          name: 'Old Name',
          external_customer_id: '1'
        });
      testCustomer = response.body;
    });

    it('should update customer fields', async () => {
      mockTilledClient.updateCustomer = jest.fn().mockResolvedValue({});

      const response = await request(app)
        .put(`/api/billing/customers/${testCustomer.id}`)
        .send({
          app_id: 'trashtech',
          email: 'new@example.com',
          name: 'New Name'
        })
        .expect(200);

      expect(response.body.email).toBe('new@example.com');
      expect(response.body.name).toBe('New Name');
    });

    it('should reject sensitive data', async () => {
      await request(app)
        .put(`/api/billing/customers/${testCustomer.id}`)
        .send({
          app_id: 'trashtech',
          email: 'test@example.com',
          card_number: '4242424242424242'
        })
        .expect(400);
    });

    it('should return 404 for non-existent customer', async () => {
      await request(app)
        .put('/api/billing/customers/99999')
        .send({
          app_id: 'trashtech',
          email: 'test@example.com'
        })
        .expect(404);
    });

    it('should return 400 when app_id missing', async () => {
      await request(app)
        .put(`/api/billing/customers/${testCustomer.id}`)
        .send({
          email: 'test@example.com'
        })
        .expect(400);
    });
  });

  describe('GET /api/billing/subscriptions/:id', () => {
    let testSubscription;

    beforeEach(async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      const customerResponse = await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech',
          email: 'test@example.com',
          name: 'Test Customer',
          external_customer_id: '1'
        });

      mockTilledClient.attachPaymentMethod.mockResolvedValue(TILLED_PAYMENT_METHOD_RESPONSE);
      mockTilledClient.createSubscription.mockResolvedValue(TILLED_SUBSCRIPTION_RESPONSE);
      const subscriptionResponse = await request(app)
        .post('/api/billing/subscriptions?app_id=trashtech')
        .send({
          billing_customer_id: customerResponse.body.id,
          payment_method_id: 'pm_test_123',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 9900
        });
      testSubscription = subscriptionResponse.body;
    });

    it('should get subscription by id', async () => {
      const response = await request(app)
        .get(`/api/billing/subscriptions/${testSubscription.id}?app_id=trashtech`)
        .expect(200);

      expect(response.body.id).toBe(testSubscription.id);
      expect(response.body.plan_id).toBe('pro-monthly');
    });

    it('should return 404 for non-existent subscription', async () => {
      await request(app)
        .get('/api/billing/subscriptions/99999?app_id=trashtech')
        .expect(404);
    });

    it('should return 404 for wrong app_id', async () => {
      await request(app)
        .get(`/api/billing/subscriptions/${testSubscription.id}?app_id=different_app`)
        .expect(404);
    });
  });

  describe('GET /api/billing/subscriptions (list with filters)', () => {
    let testCustomer;

    beforeEach(async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      const customerResponse = await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech',
          email: 'test@example.com',
          name: 'Test Customer',
          external_customer_id: '1'
        });
      testCustomer = customerResponse.body;

      mockTilledClient.attachPaymentMethod.mockResolvedValue(TILLED_PAYMENT_METHOD_RESPONSE);
      mockTilledClient.createSubscription.mockResolvedValue(TILLED_SUBSCRIPTION_RESPONSE);
      await request(app)
        .post('/api/billing/subscriptions?app_id=trashtech')
        .send({
          billing_customer_id: testCustomer.id,
          payment_method_id: 'pm_test_123',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 9900
        });
    });

    it('should list subscriptions by billing_customer_id', async () => {
      const response = await request(app)
        .get(`/api/billing/subscriptions?app_id=trashtech&billing_customer_id=${testCustomer.id}`)
        .expect(200);

      expect(Array.isArray(response.body)).toBe(true);
      expect(response.body.length).toBeGreaterThan(0);
      expect(response.body[0].billing_customer_id).toBe(testCustomer.id);
    });

    it('should list subscriptions by app_id', async () => {
      const response = await request(app)
        .get('/api/billing/subscriptions?app_id=trashtech')
        .expect(200);

      expect(Array.isArray(response.body)).toBe(true);
      expect(response.body.length).toBeGreaterThan(0);
    });

    it('should list subscriptions by status', async () => {
      const response = await request(app)
        .get('/api/billing/subscriptions?app_id=trashtech&status=active')
        .expect(200);

      expect(Array.isArray(response.body)).toBe(true);
    });

    it('should return empty array when no matches', async () => {
      const response = await request(app)
        .get('/api/billing/subscriptions?app_id=trashtech&billing_customer_id=99999')
        .expect(200);

      expect(Array.isArray(response.body)).toBe(true);
      expect(response.body.length).toBe(0);
    });
  });

  describe('PUT /api/billing/subscriptions/:id', () => {
    let testSubscription;

    beforeEach(async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);
      const customerResponse = await request(app)
        .post('/api/billing/customers')
        .send({
          app_id: 'trashtech',
          email: 'test@example.com',
          name: 'Test Customer',
          external_customer_id: '1'
        });

      mockTilledClient.attachPaymentMethod.mockResolvedValue(TILLED_PAYMENT_METHOD_RESPONSE);
      mockTilledClient.createSubscription.mockResolvedValue(TILLED_SUBSCRIPTION_RESPONSE);
      const subscriptionResponse = await request(app)
        .post('/api/billing/subscriptions?app_id=trashtech')
        .send({
          billing_customer_id: customerResponse.body.id,
          payment_method_id: 'pm_test_123',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 9900
        });
      testSubscription = subscriptionResponse.body;
    });

    it('should update subscription metadata', async () => {
      mockTilledClient.updateSubscription = jest.fn().mockResolvedValue({});

      const response = await request(app)
        .put(`/api/billing/subscriptions/${testSubscription.id}`)
        .send({
          app_id: 'trashtech',
          metadata: { feature: 'premium' }
        })
        .expect(200);

      expect(response.body.metadata).toEqual({ feature: 'premium' });
    });

    it('should update subscription plan fields', async () => {
      const response = await request(app)
        .put(`/api/billing/subscriptions/${testSubscription.id}`)
        .send({
          app_id: 'trashtech',
          plan_name: 'Pro Monthly Updated',
          price_cents: 10900
        })
        .expect(200);

      expect(response.body.plan_name).toBe('Pro Monthly Updated');
      expect(response.body.price_cents).toBe(10900);
    });

    it('should reject billing cycle changes', async () => {
      const response = await request(app)
        .put(`/api/billing/subscriptions/${testSubscription.id}`)
        .send({
          app_id: 'trashtech',
          interval_unit: 'year'
        })
        .expect(400);

      expect(response.body.error).toContain('Cannot change billing cycle');
    });

    it('should return 404 for non-existent subscription', async () => {
      await request(app)
        .put('/api/billing/subscriptions/99999')
        .send({
          app_id: 'trashtech',
          metadata: {}
        })
        .expect(404);
    });

    it('should return 400 when app_id missing', async () => {
      await request(app)
        .put(`/api/billing/subscriptions/${testSubscription.id}`)
        .send({
          metadata: {}
        })
        .expect(400);
    });
  });

  describe('POST /api/billing/charges/one-time', () => {
    let testCustomer;

    beforeEach(async () => {
      // Create a customer with default payment method
      testCustomer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'charge_test_customer',
          tilled_customer_id: 'tc_charge_test',
          email: 'charge@test.com',
          name: 'Charge Test Customer',
          default_payment_method_id: 'pm_test_123',
        },
      });

      // Mock Tilled charge creation
      mockTilledClient.createCharge = jest.fn().mockResolvedValue({
        id: 'ch_test_123',
        status: 'succeeded',
      });
    });

    it('should return 400 if app_id missing', async () => {
      const response = await request(app)
        .post('/api/billing/charges/one-time')
        .set('Idempotency-Key', 'test-key-1')
        .send({
          external_customer_id: 'charge_test_customer',
          amount_cents: 3500,
          reason: 'extra_pickup',
          reference_id: 'pickup_001',
        })
        .expect(400);

      expect(response.body.error).toContain('app_id');
    });

    it('should return 400 if Idempotency-Key missing', async () => {
      const response = await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .send({
          external_customer_id: 'charge_test_customer',
          amount_cents: 3500,
          reason: 'extra_pickup',
          reference_id: 'pickup_002',
        })
        .expect(400);

      expect(response.body.error).toBe('Validation failed');
      expect(response.body.details).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            field: 'idempotency-key',
            message: expect.stringContaining('Idempotency-Key')
          })
        ])
      );
    });

    it('should return 400 if required fields missing', async () => {
      const response = await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-2')
        .send({
          external_customer_id: 'charge_test_customer',
          // Missing amount_cents, reason, reference_id
        })
        .expect(400);

      expect(response.body.error).toBeDefined();
    });

    it('should return 404 if external_customer_id not found', async () => {
      await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-3')
        .send({
          external_customer_id: 'nonexistent_customer',
          amount_cents: 3500,
          reason: 'extra_pickup',
          reference_id: 'pickup_003',
        })
        .expect(404);
    });

    it('should return 409 if no default payment method', async () => {
      // Create customer without default payment method
      const customerNoPayment = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'no_payment_customer',
          tilled_customer_id: 'tc_no_payment',
          email: 'nopayment@test.com',
          name: 'No Payment Customer',
          default_payment_method_id: null,
        },
      });

      await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-4')
        .send({
          external_customer_id: 'no_payment_customer',
          amount_cents: 3500,
          reason: 'extra_pickup',
          reference_id: 'pickup_004',
        })
        .expect(409);
    });

    it('should create one-time charge and persist record', async () => {
      const response = await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-5')
        .send({
          external_customer_id: 'charge_test_customer',
          amount_cents: 3500,
          currency: 'usd',
          reason: 'extra_pickup',
          reference_id: 'pickup_005',
          service_date: '2026-01-23',
          note: 'Extra pickup requested',
          metadata: { route_id: 'R12' },
        });

      expect(response.status).toBe(201);
      expect(response.body.charge).toBeDefined();
      expect(response.body.charge.status).toBe('succeeded');
      expect(response.body.charge.amount_cents).toBe(3500);
      expect(response.body.charge.reason).toBe('extra_pickup');
      expect(response.body.charge.reference_id).toBe('pickup_005');
      expect(response.body.charge.tilled_charge_id).toBe('ch_test_123');

      // Verify DB record
      const dbCharge = await billingPrisma.billing_charges.findFirst({
        where: {
          app_id: 'trashtech',
          reference_id: 'pickup_005',
        },
      });

      expect(dbCharge).not.toBeNull();
      expect(dbCharge.status).toBe('succeeded');
      expect(dbCharge.amount_cents).toBe(3500);
      expect(dbCharge.note).toBe('Extra pickup requested');
      expect(dbCharge.metadata).toEqual({ route_id: 'R12' });

      // Verify Tilled was called
      expect(mockTilledClient.createCharge).toHaveBeenCalledWith({
        appId: 'trashtech',
        tilledCustomerId: 'tc_charge_test',
        paymentMethodId: 'pm_test_123',
        amountCents: 3500,
        currency: 'usd',
        description: 'extra_pickup',
        metadata: expect.any(Object),
      });
    });

    it('should return existing charge for duplicate reference_id and not double charge', async () => {
      // First request
      const firstResponse = await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-6')
        .send({
          external_customer_id: 'charge_test_customer',
          amount_cents: 3500,
          reason: 'extra_pickup',
          reference_id: 'pickup_duplicate',
        })
        .expect(201);

      expect(firstResponse.body.charge.reference_id).toBe('pickup_duplicate');
      const firstChargeId = firstResponse.body.charge.id;

      // Reset mock call count
      mockTilledClient.createCharge.mockClear();

      // Second request with DIFFERENT idempotency key but SAME reference_id
      const secondResponse = await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-7-different')
        .send({
          external_customer_id: 'charge_test_customer',
          amount_cents: 3500,
          reason: 'extra_pickup',
          reference_id: 'pickup_duplicate', // SAME reference_id
        })
        .expect(201);

      // Should return existing charge
      expect(secondResponse.body.charge.id).toBe(firstChargeId);
      expect(secondResponse.body.charge.reference_id).toBe('pickup_duplicate');

      // Verify Tilled was NOT called again
      expect(mockTilledClient.createCharge).not.toHaveBeenCalled();

      // Verify only one DB record exists
      const allCharges = await billingPrisma.billing_charges.findMany({
        where: {
          app_id: 'trashtech',
          reference_id: 'pickup_duplicate',
        },
      });

      expect(allCharges).toHaveLength(1);
    });

    it('should reject PCI-sensitive fields', async () => {
      const response = await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-8')
        .send({
          external_customer_id: 'charge_test_customer',
          amount_cents: 3500,
          reason: 'extra_pickup',
          reference_id: 'pickup_006',
          card_number: '4242424242424242', // PCI-sensitive
        })
        .expect(400);

      expect(response.body.error).toContain('PCI violation');
    });

    it('should handle tip charges', async () => {
      const response = await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-9')
        .send({
          external_customer_id: 'charge_test_customer',
          amount_cents: 500,
          reason: 'tip',
          reference_id: 'tip_001',
          note: 'Driver tip',
        })
        .expect(201);

      expect(response.body.charge.reason).toBe('tip');
      expect(response.body.charge.amount_cents).toBe(500);
    });

    it('should reject empty reference_id', async () => {
      await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-empty-ref')
        .send({
          external_customer_id: 'charge_test_customer',
          amount_cents: 3500,
          reason: 'extra_pickup',
          reference_id: '', // Empty string
        })
        .expect(400);
    });

    it('should reject whitespace-only reference_id', async () => {
      await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-whitespace-ref')
        .send({
          external_customer_id: 'charge_test_customer',
          amount_cents: 3500,
          reason: 'extra_pickup',
          reference_id: '   ', // Whitespace only
        })
        .expect(400);
    });

    it('should set charge_type to "one_time"', async () => {
      const response = await request(app)
        .post('/api/billing/charges/one-time?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-charge-type')
        .send({
          external_customer_id: 'charge_test_customer',
          amount_cents: 3500,
          reason: 'extra_pickup',
          reference_id: 'pickup_charge_type_test',
        })
        .expect(201);

      expect(response.body.charge.charge_type).toBe('one_time');

      // Verify in DB
      const dbCharge = await billingPrisma.billing_charges.findFirst({
        where: {
          app_id: 'trashtech',
          reference_id: 'pickup_charge_type_test',
        },
      });

      expect(dbCharge.charge_type).toBe('one_time');
    });
  });

  // =========================================================
  // Webhook Retry & Admin Endpoints
  // =========================================================

  describe('Webhook Retry Flow', () => {
    it('should store payload on webhook create', async () => {
      const event = {
        id: 'evt_payload_test_' + Date.now(),
        type: 'subscription.created',
        data: { object: TILLED_SUBSCRIPTION_RESPONSE }
      };
      const signature = generateWebhookSignature(event, 'whsec_123');
      mockTilledClient.verifyWebhookSignature.mockReturnValue(true);

      await request(app)
        .post('/api/billing/webhooks/trashtech')
        .set('payments-signature', signature)
        .send(event)
        .expect(200);

      const webhook = await billingPrisma.billing_webhooks.findFirst({
        where: { event_id: event.id, app_id: 'trashtech' }
      });
      expect(webhook.payload).toBeTruthy();
      expect(webhook.payload.id).toBe(event.id);
      expect(webhook.payload.type).toBe(event.type);
    });

    it('should set retry fields on handler failure', async () => {
      // Use dispute.created which will throw on upsert if given malformed data
      // (missing required 'status' field causes a Prisma error)
      const event = {
        id: 'evt_fail_retry_' + Date.now(),
        type: 'dispute.created',
        data: {
          object: {
            id: 'dsp_test_fail',
            // Omit 'status' to trigger a Prisma validation error on create
            payment_intent_id: null,
            charge_id: null
          }
        }
      };
      const signature = generateWebhookSignature(event, 'whsec_123');
      mockTilledClient.verifyWebhookSignature.mockReturnValue(true);

      // This should fail because dispute upsert requires status
      await request(app)
        .post('/api/billing/webhooks/trashtech')
        .set('payments-signature', signature)
        .send(event)
        .expect(500);

      const webhook = await billingPrisma.billing_webhooks.findFirst({
        where: { event_id: event.id, app_id: 'trashtech' }
      });
      expect(webhook.status).toBe('failed');
      expect(webhook.next_attempt_at).toBeTruthy();
      expect(webhook.last_attempt_at).toBeTruthy();
      expect(webhook.error).toBeTruthy();
      expect(webhook.error_code).toBeTruthy();
      expect(webhook.dead_at).toBeNull();

      // Check attempt was recorded
      const attempts = await billingPrisma.billing_webhook_attempts.findMany({
        where: { event_id: event.id, app_id: 'trashtech' }
      });
      expect(attempts).toHaveLength(1);
      expect(attempts[0].attempt_number).toBe(1);
      expect(attempts[0].status).toBe('failed');
    });
  });

  describe('POST /api/billing/webhook-admin/retry', () => {
    it('should process retryable webhooks', async () => {
      // Seed a failed webhook with next_attempt_at in the past
      await billingPrisma.billing_webhooks.create({
        data: {
          app_id: 'trashtech',
          event_id: 'evt_retry_admin_' + Date.now(),
          event_type: 'subscription.created',
          status: 'failed',
          payload: {
            id: 'evt_retry_admin_' + Date.now(),
            type: 'subscription.created',
            data: { object: TILLED_SUBSCRIPTION_RESPONSE }
          },
          attempt_count: 1,
          next_attempt_at: new Date(Date.now() - 60000),
          last_attempt_at: new Date(Date.now() - 120000),
          error: 'Temporary error'
        }
      });

      const response = await request(app)
        .post('/api/billing/webhook-admin/retry?app_id=trashtech')
        .send({})
        .expect(200);

      expect(response.body.processed).toBeGreaterThanOrEqual(1);
      expect(response.body.results).toBeInstanceOf(Array);
    });

    it('should return empty results when no retryable webhooks', async () => {
      const response = await request(app)
        .post('/api/billing/webhook-admin/retry?app_id=trashtech')
        .send({})
        .expect(200);

      expect(response.body.processed).toBe(0);
      expect(response.body.results).toEqual([]);
    });
  });

  describe('GET /api/billing/webhook-admin/stats', () => {
    it('should return retry queue stats', async () => {
      const response = await request(app)
        .get('/api/billing/webhook-admin/stats?app_id=trashtech')
        .expect(200);

      expect(response.body).toHaveProperty('failed');
      expect(response.body).toHaveProperty('processing');
      expect(response.body).toHaveProperty('deadLettered');
      expect(response.body).toHaveProperty('pendingRetries');
      expect(response.body).toHaveProperty('totalProcessed');
      expect(typeof response.body.failed).toBe('number');
    });

    it('should reflect correct counts after webhook processing', async () => {
      // Seed some data
      const now = new Date();
      await billingPrisma.billing_webhooks.create({
        data: {
          app_id: 'trashtech',
          event_id: 'evt_stats_processed_' + Date.now(),
          event_type: 'subscription.created',
          status: 'processed',
          processed_at: now
        }
      });
      await billingPrisma.billing_webhooks.create({
        data: {
          app_id: 'trashtech',
          event_id: 'evt_stats_dead_' + Date.now(),
          event_type: 'subscription.updated',
          status: 'failed',
          dead_at: now,
          error: 'Max retries exceeded'
        }
      });

      const response = await request(app)
        .get('/api/billing/webhook-admin/stats?app_id=trashtech')
        .expect(200);

      expect(response.body.totalProcessed).toBeGreaterThanOrEqual(1);
      expect(response.body.deadLettered).toBeGreaterThanOrEqual(1);
    });
  });

  describe('POST /api/billing/webhook-admin/retry/:event_id', () => {
    it('should retry a dead-lettered webhook', async () => {
      // Use an unhandled event type so handleWebhookEvent does a no-op (logs and returns)
      const eventId = 'evt_dead_admin_' + Date.now();
      await billingPrisma.billing_webhooks.create({
        data: {
          app_id: 'trashtech',
          event_id: eventId,
          event_type: 'custom.unhandled_event',
          status: 'failed',
          payload: {
            id: eventId,
            type: 'custom.unhandled_event',
            data: { object: { foo: 'bar' } }
          },
          attempt_count: 5,
          dead_at: new Date(),
          error: 'Persistent failure'
        }
      });

      const response = await request(app)
        .post(`/api/billing/webhook-admin/retry/${eventId}?app_id=trashtech`)
        .send({});

      expect(response.status).toBe(200);
      expect(response.body).toHaveProperty('eventId', eventId);
      expect(response.body).toHaveProperty('status');
      expect(response.body.status).toBe('processed');
      expect(response.body).toHaveProperty('attempt', 6);

      // Verify webhook is now processed in DB
      const webhook = await billingPrisma.billing_webhooks.findFirst({
        where: { event_id: eventId, app_id: 'trashtech' }
      });
      expect(webhook.status).toBe('processed');
      expect(webhook.dead_at).toBeNull();
    });

    it('should return error for non-existent webhook', async () => {
      await request(app)
        .post('/api/billing/webhook-admin/retry/evt_nonexistent?app_id=trashtech')
        .send({})
        .expect(500);
    });
  });
});
