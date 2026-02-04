const request = require('supertest');
const express = require('express');
const routes = require('../../backend/src/routes/index');
const handleBillingError = require('../../backend/src/middleware/errorHandler');
const { billingPrisma } = require('../../backend/src/prisma');
const { cleanDatabase, setupIntegrationTests, teardownIntegrationTests } = require('./database-cleanup');
const TilledClient = require('../../backend/src/tilledClient');

// Mock TilledClient
jest.mock('../../backend/src/tilledClient');

describe('Phase 1 Routes Integration', () => {
  let app;
  let mockTilledClient;

  beforeAll(async () => {
    await setupIntegrationTests();

    app = express();
    app.use(express.json());
    app.use('/api/billing', routes);
    app.use(handleBillingError); // Error handler MUST be mounted last

    mockTilledClient = {
      attachPaymentMethod: jest.fn(),
      detachPaymentMethod: jest.fn(),
      getPaymentMethod: jest.fn(),
      updateSubscription: jest.fn(),
      cancelSubscription: jest.fn(),
      createSubscription: jest.fn()
    };
    TilledClient.mockImplementation(() => mockTilledClient);
  });

  beforeEach(async () => {
    // Clean up database using centralized cleanup (wrapped in transaction)
    await cleanDatabase();
    jest.clearAllMocks();
  });

  afterAll(async () => {
    await teardownIntegrationTests();
  });

  describe('GET /api/billing/state', () => {
    it('returns composed billing state for customer', async () => {
      // Setup test data
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_123',
          tilled_customer_id: 'cus_test',
          email: 'test@example.com',
          name: 'Test Customer',
          default_payment_method_id: 'pm_test',
          payment_method_type: 'card'
        }
      });

      const subscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub_test',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 9900,
          status: 'active',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date(),
          current_period_end: new Date(Date.now() + 30 * 24 * 60 * 60 * 1000),
          payment_method_id: 'pm_test',
          payment_method_type: 'card'
        }
      });

      const pm = await billingPrisma.billing_payment_methods.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_payment_method_id: 'pm_test',
          type: 'card',
          brand: 'visa',
          last4: '4242',
          exp_month: 12,
          exp_year: 2028,
          is_default: true
        }
      });

      // Set env for entitlements
      process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH = JSON.stringify({
        'pro-monthly': { analytics: true, max_trucks: 10 }
      });

      const response = await request(app)
        .get('/api/billing/state')
        .query({ app_id: 'trashtech', external_customer_id: 'ext_123' });

      expect(response.status).toBe(200);
      expect(response.body.customer.email).toBe('test@example.com');
      expect(response.body.subscription.status).toBe('active');
      expect(response.body.payment.has_default_payment_method).toBe(true);
      expect(response.body.payment.default_payment_method.last4).toBe('4242');
      expect(response.body.access.is_active).toBe(true);
      expect(response.body.access.access_state).toBe('full');
      expect(response.body.entitlements.features.analytics).toBe(true);

      delete process.env.BILLING_ENTITLEMENTS_JSON_TRASHTECH;
    });

    it('returns 404 when customer not found', async () => {
      const response = await request(app)
        .get('/api/billing/state')
        .query({ app_id: 'trashtech', external_customer_id: 'nonexistent' });

      expect(response.status).toBe(404);
      expect(response.body.error).toContain('not found');
    });
  });

  describe('Payment Methods CRUD', () => {
    let customer;

    beforeEach(async () => {
      customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_123',
          tilled_customer_id: 'cus_test',
          email: 'test@example.com',
          name: 'Test Customer'
        }
      });
    });

    describe('GET /api/billing/payment-methods', () => {
      it('lists payment methods for customer', async () => {
        await billingPrisma.billing_payment_methods.createMany({
          data: [
            {
              app_id: 'trashtech',
              billing_customer_id: customer.id,
              tilled_payment_method_id: 'pm_1',
              type: 'card',
              brand: 'visa',
              last4: '4242',
              is_default: true
            },
            {
              app_id: 'trashtech',
              billing_customer_id: customer.id,
              tilled_payment_method_id: 'pm_2',
              type: 'card',
              brand: 'mastercard',
              last4: '5555',
              is_default: false
            }
          ]
        });

        const response = await request(app)
          .get('/api/billing/payment-methods')
          .query({ app_id: 'trashtech', billing_customer_id: customer.id });

        expect(response.status).toBe(200);
        expect(response.body.payment_methods).toHaveLength(2);
        expect(response.body.payment_methods[0].is_default).toBe(true);
      });
    });

    describe('POST /api/billing/payment-methods', () => {
      it('adds payment method to customer', async () => {
        mockTilledClient.attachPaymentMethod.mockResolvedValue({ id: 'pm_new' });
        mockTilledClient.getPaymentMethod.mockResolvedValue({
          id: 'pm_new',
          type: 'card',
          card: { brand: 'visa', last4: '4242', exp_month: 12, exp_year: 2028 }
        });

        const response = await request(app)
          .post('/api/billing/payment-methods')
          .send({
            app_id: 'trashtech',
            billing_customer_id: customer.id,
            payment_method_id: 'pm_new'
          });

        expect(response.status).toBe(201);
        expect(response.body.tilled_payment_method_id).toBe('pm_new');
        expect(response.body.type).toBe('card');
        expect(response.body.last4).toBe('4242');
      });
    });

    describe('PUT /api/billing/payment-methods/:id/default', () => {
      it('sets payment method as default', async () => {
        const pm = await billingPrisma.billing_payment_methods.create({
          data: {
            app_id: 'trashtech',
            billing_customer_id: customer.id,
            tilled_payment_method_id: 'pm_test',
            type: 'card',
            brand: 'visa',
            last4: '4242',
            is_default: false
          }
        });

        const response = await request(app)
          .put('/api/billing/payment-methods/pm_test/default')
          .send({
            app_id: 'trashtech',
            billing_customer_id: customer.id
          });

        expect(response.status).toBe(200);
        expect(response.body.is_default).toBe(true);

        // Verify customer fast-path updated
        const updatedCustomer = await billingPrisma.billing_customers.findUnique({
          where: { id: customer.id }
        });
        expect(updatedCustomer.default_payment_method_id).toBe('pm_test');
      });
    });

    describe('DELETE /api/billing/payment-methods/:id', () => {
      it('soft-deletes payment method', async () => {
        const pm = await billingPrisma.billing_payment_methods.create({
          data: {
            app_id: 'trashtech',
            billing_customer_id: customer.id,
            tilled_payment_method_id: 'pm_test',
            type: 'card',
            brand: 'visa',
            last4: '4242',
            is_default: true
          }
        });

        await billingPrisma.billing_customers.update({
          where: { id: customer.id },
          data: {
            default_payment_method_id: 'pm_test',
            payment_method_type: 'card'
          }
        });

        mockTilledClient.detachPaymentMethod.mockResolvedValue({});

        const response = await request(app)
          .delete('/api/billing/payment-methods/pm_test')
          .query({
            app_id: 'trashtech',
            billing_customer_id: customer.id
          });

        expect(response.status).toBe(200);
        expect(response.body.deleted).toBe(true);

        // Verify soft deleted
        const deletedPM = await billingPrisma.billing_payment_methods.findUnique({
          where: { tilled_payment_method_id: 'pm_test' }
        });
        expect(deletedPM.deleted_at).not.toBeNull();

        // Verify customer default cleared
        const updatedCustomer = await billingPrisma.billing_customers.findUnique({
          where: { id: customer.id }
        });
        expect(updatedCustomer.default_payment_method_id).toBeNull();
      });
    });
  });

  describe('Subscription Lifecycle', () => {
    let customer, subscription;

    beforeEach(async () => {
      customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_123',
          tilled_customer_id: 'cus_test',
          email: 'test@example.com',
          name: 'Test Customer'
        }
      });

      subscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub_test',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 9900,
          status: 'active',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date(),
          current_period_end: new Date(Date.now() + 30 * 24 * 60 * 60 * 1000),
          payment_method_id: 'pm_test',
          payment_method_type: 'card'
        }
      });
    });

    describe('DELETE /api/billing/subscriptions/:id with at_period_end', () => {
      it('sets cancel_at_period_end without immediate cancellation', async () => {
        mockTilledClient.updateSubscription.mockResolvedValue({
          cancel_at_period_end: true
        });

        const response = await request(app)
          .delete(`/api/billing/subscriptions/${subscription.id}`)
          .query({ app_id: 'trashtech', at_period_end: 'true' });

        expect(response.status).toBe(200);
        expect(response.body.cancel_at_period_end).toBe(true);
        expect(response.body.status).toBe('active');  // Still active
      });

      it('immediately cancels when at_period_end=false', async () => {
        mockTilledClient.cancelSubscription.mockResolvedValue({
          status: 'canceled',
          canceled_at: Math.floor(Date.now() / 1000)
        });

        const response = await request(app)
          .delete(`/api/billing/subscriptions/${subscription.id}`)
          .query({ app_id: 'trashtech', at_period_end: 'false' });

        expect(response.status).toBe(200);
        expect(response.body.status).toBe('canceled');
        expect(response.body.canceled_at).toBeTruthy();
        expect(response.body.ended_at).toBeTruthy();
      });
    });

    describe('POST /api/billing/subscriptions/change-cycle', () => {
      it('cancels old and creates new subscription', async () => {
        mockTilledClient.attachPaymentMethod.mockResolvedValue({ id: 'pm_test' });
        mockTilledClient.createSubscription.mockResolvedValue({
          id: 'sub_new',
          status: 'active',
          current_period_start: Math.floor(Date.now() / 1000),
          current_period_end: Math.floor(Date.now() / 1000) + 31536000
        });
        mockTilledClient.cancelSubscription.mockResolvedValue({
          status: 'canceled',
          canceled_at: Math.floor(Date.now() / 1000)
        });

        const response = await request(app)
          .post('/api/billing/subscriptions/change-cycle')
          .send({
            app_id: 'trashtech',
            billing_customer_id: customer.id,
            from_subscription_id: subscription.id,
            new_plan_id: 'pro-annual',
            new_plan_name: 'Pro Annual',
            price_cents: 99900,
            payment_method_id: 'pm_test',
            payment_method_type: 'card',
            options: {
              intervalUnit: 'year',
              intervalCount: 1
            }
          });

        expect(response.status).toBe(201);
        expect(response.body.canceled_subscription.status).toBe('canceled');
        expect(response.body.new_subscription.plan_id).toBe('pro-annual');
        expect(response.body.new_subscription.price_cents).toBe(99900);

        // Verify old subscription canceled in DB
        const oldSub = await billingPrisma.billing_subscriptions.findUnique({
          where: { id: subscription.id }
        });
        expect(oldSub.status).toBe('canceled');
      });

      it('returns 404 when subscription not in app scope', async () => {
        const response = await request(app)
          .post('/api/billing/subscriptions/change-cycle')
          .send({
            app_id: 'trashtech',
            billing_customer_id: customer.id,
            from_subscription_id: 999999,
            new_plan_id: 'pro-annual',
            new_plan_name: 'Pro Annual',
            price_cents: 99900,
            payment_method_id: 'pm_test',
            payment_method_type: 'card'
          });

        expect(response.status).toBe(404);
        expect(response.body.error).toContain('not found');
      });
    });
  });
});
