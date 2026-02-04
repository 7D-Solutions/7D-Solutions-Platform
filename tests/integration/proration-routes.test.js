const request = require('supertest');
const express = require('express');
const routes = require('../../backend/src/routes/index');
const handleBillingError = require('../../backend/src/middleware/errorHandler');
const { billingPrisma } = require('../../backend/src/prisma');
const { cleanDatabase, setupIntegrationTests, teardownIntegrationTests } = require('./database-cleanup');
const TilledClient = require('../../backend/src/tilledClient');

// Mock TilledClient
jest.mock('../../backend/src/tilledClient');

describe('Phase 3 Proration Routes Integration', () => {
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

  describe('POST /api/billing/proration/calculate', () => {
    it('calculates proration preview for subscription upgrade', async () => {
      // Create test customer and subscription
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_proration_001',
          tilled_customer_id: 'cus_test_proration',
          email: 'proration@example.com',
          name: 'Proration Test Customer'
        }
      });

      const subscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub_test_proration',
          plan_id: 'basic-monthly',
          plan_name: 'Basic Monthly',
          price_cents: 5000, // $50
          status: 'active',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date('2026-01-01T00:00:00Z'),
          current_period_end: new Date('2026-02-01T00:00:00Z'), // 31-day January
          payment_method_id: 'pm_test',
          payment_method_type: 'card'
        }
      });

      const response = await request(app)
        .post('/api/billing/proration/calculate')
        .query({ app_id: 'trashtech' })
        .send({
          subscription_id: subscription.id,
          change_date: '2026-01-15T00:00:00Z', // Mid-cycle (day 15 of 31)
          new_price_cents: 10000, // $100 upgrade
          old_price_cents: 5000, // $50 current
          proration_behavior: 'create_prorations'
        });

      expect(response.status).toBe(200);
      expect(response.body.proration).toBeDefined();
      expect(response.body.proration.subscription_id).toBe(subscription.id);
      expect(response.body.proration.time_proration.daysUsed).toBe(14); // Jan 1-14 inclusive
      expect(response.body.proration.time_proration.daysRemaining).toBe(17); // Jan 15-31 inclusive
      expect(response.body.proration.time_proration.daysTotal).toBe(31);
      expect(response.body.proration.old_plan.credit_cents).toBeGreaterThan(0);
      expect(response.body.proration.new_plan.charge_cents).toBeGreaterThan(0);
      expect(response.body.proration.net_change.amount_cents).toBeGreaterThan(0);
      expect(response.body.proration.net_change.type).toBe('charge');
    });

    it('calculates proration preview for subscription downgrade', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_proration_002',
          tilled_customer_id: 'cus_test_proration2',
          email: 'proration2@example.com',
          name: 'Proration Test Customer 2'
        }
      });

      const subscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub_test_proration2',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 15000, // $150
          status: 'active',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date('2026-01-01T00:00:00Z'),
          current_period_end: new Date('2026-02-01T00:00:00Z'),
          payment_method_id: 'pm_test',
          payment_method_type: 'card'
        }
      });

      const response = await request(app)
        .post('/api/billing/proration/calculate')
        .query({ app_id: 'trashtech' })
        .send({
          subscription_id: subscription.id,
          change_date: '2026-01-15T00:00:00Z',
          new_price_cents: 5000, // $50 downgrade
          old_price_cents: 15000, // $150 current
          proration_behavior: 'create_prorations'
        });

      expect(response.status).toBe(200);
      expect(response.body.proration.net_change.amount_cents).toBeLessThan(0);
      expect(response.body.proration.net_change.type).toBe('credit');
    });

    it('validates required parameters', async () => {
      const response = await request(app)
        .post('/api/billing/proration/calculate')
        .query({ app_id: 'trashtech' })
        .send({
          // Missing required fields
          subscription_id: 999,
          change_date: 'invalid-date'
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
    });

    it('returns 404 for non-existent subscription', async () => {
      const response = await request(app)
        .post('/api/billing/proration/calculate')
        .query({ app_id: 'trashtech' })
        .send({
          subscription_id: 999999,
          change_date: '2026-01-15T00:00:00Z',
          new_price_cents: 10000,
          old_price_cents: 5000
        });

      expect(response.status).toBe(404);
      expect(response.body.error).toContain('not found');
    });
  });

  describe('POST /api/billing/subscriptions/:subscription_id/proration/apply', () => {
    it('applies subscription upgrade with proration charges', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_proration_apply_001',
          tilled_customer_id: 'cus_test_apply',
          email: 'apply@example.com',
          name: 'Apply Test Customer'
        }
      });

      const subscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub_test_apply',
          plan_id: 'basic-monthly',
          plan_name: 'Basic Monthly',
          price_cents: 5000, // $50
          status: 'active',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date('2026-01-01T00:00:00Z'),
          current_period_end: new Date('2026-02-01T00:00:00Z'),
          payment_method_id: 'pm_test',
          payment_method_type: 'card'
        }
      });

      const response = await request(app)
        .post(`/api/billing/subscriptions/${subscription.id}/proration/apply`)
        .query({ app_id: 'trashtech' })
        .send({
          new_price_cents: 10000, // $100
          old_price_cents: 5000, // $50
          proration_behavior: 'create_prorations',
          effective_date: '2026-01-15T00:00:00Z'
        });

      expect(response.status).toBe(200);
      expect(response.body.subscription).toBeDefined();
      expect(response.body.proration).toBeDefined();
      expect(response.body.charges).toBeDefined();
      expect(response.body.subscription.price_cents).toBe(10000);

      // Verify charges were created
      expect(response.body.charges.length).toBeGreaterThan(0);

      // Verify subscription was updated in database
      const updatedSubscription = await billingPrisma.billing_subscriptions.findUnique({
        where: { id: subscription.id }
      });
      expect(updatedSubscription.price_cents).toBe(10000);
      expect(updatedSubscription.metadata.last_change.proration_applied).toBe(true);
    });

    it('applies subscription change with proration behavior "none"', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_proration_none_001',
          tilled_customer_id: 'cus_test_none',
          email: 'none@example.com',
          name: 'None Test Customer'
        }
      });

      const subscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub_test_none',
          plan_id: 'basic-monthly',
          plan_name: 'Basic Monthly',
          price_cents: 5000,
          status: 'active',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date('2026-01-01T00:00:00Z'),
          current_period_end: new Date('2026-02-01T00:00:00Z'),
          payment_method_id: 'pm_test',
          payment_method_type: 'card'
        }
      });

      const response = await request(app)
        .post(`/api/billing/subscriptions/${subscription.id}/proration/apply`)
        .query({ app_id: 'trashtech' })
        .send({
          new_price_cents: 10000,
          old_price_cents: 5000,
          proration_behavior: 'none',
          effective_date: '2026-01-15T00:00:00Z'
        });

      expect(response.status).toBe(200);
      expect(response.body.proration).toBeNull();
      expect(response.body.charges).toEqual([]);

      // Subscription should be updated but no charges created
      const updatedSubscription = await billingPrisma.billing_subscriptions.findUnique({
        where: { id: subscription.id }
      });
      expect(updatedSubscription.price_cents).toBe(10000);
    });

    it('validates required parameters for apply endpoint', async () => {
      const response = await request(app)
        .post('/api/billing/subscriptions/999/proration/apply')
        .query({ app_id: 'trashtech' })
        .send({
          // Missing required fields
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
    });
  });

  describe('POST /api/billing/subscriptions/:subscription_id/proration/cancellation-refund', () => {
    it('calculates cancellation refund for mid-cycle cancellation', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_cancel_001',
          tilled_customer_id: 'cus_test_cancel',
          email: 'cancel@example.com',
          name: 'Cancel Test Customer'
        }
      });

      const subscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub_test_cancel',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 15000, // $150
          status: 'active',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date('2026-01-01T00:00:00Z'),
          current_period_end: new Date('2026-02-01T00:00:00Z'),
          payment_method_id: 'pm_test',
          payment_method_type: 'card'
        }
      });

      const response = await request(app)
        .post(`/api/billing/subscriptions/${subscription.id}/proration/cancellation-refund`)
        .query({ app_id: 'trashtech' })
        .send({
          cancellation_date: '2026-01-15T00:00:00Z',
          refund_behavior: 'partial_refund'
        });

      expect(response.status).toBe(200);
      expect(response.body.cancellation_refund).toBeDefined();
      expect(response.body.cancellation_refund.subscription_id).toBe(subscription.id);
      expect(response.body.cancellation_refund.refund_amount_cents).toBeGreaterThan(0);
      expect(response.body.cancellation_refund.action).toBe('refund');
    });

    it('calculates account credit instead of refund', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_credit_001',
          tilled_customer_id: 'cus_test_credit',
          email: 'credit@example.com',
          name: 'Credit Test Customer'
        }
      });

      const subscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub_test_credit',
          plan_id: 'basic-monthly',
          plan_name: 'Basic Monthly',
          price_cents: 5000,
          status: 'active',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date('2026-01-01T00:00:00Z'),
          current_period_end: new Date('2026-02-01T00:00:00Z'),
          payment_method_id: 'pm_test',
          payment_method_type: 'card'
        }
      });

      const response = await request(app)
        .post(`/api/billing/subscriptions/${subscription.id}/proration/cancellation-refund`)
        .query({ app_id: 'trashtech' })
        .send({
          cancellation_date: '2026-01-15T00:00:00Z',
          refund_behavior: 'account_credit'
        });

      expect(response.status).toBe(200);
      expect(response.body.cancellation_refund.action).toBe('account_credit');
    });

    it('validates cancellation parameters', async () => {
      const response = await request(app)
        .post('/api/billing/subscriptions/999/proration/cancellation-refund')
        .query({ app_id: 'trashtech' })
        .send({
          cancellation_date: 'invalid-date'
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
    });
  });
});