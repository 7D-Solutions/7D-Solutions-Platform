const request = require('supertest');
const express = require('express');
const routes = require('../../backend/src/routes/index');
const handleBillingError = require('../../backend/src/middleware/errorHandler');
const { billingPrisma } = require('../../backend/src/prisma');
const { cleanDatabase, setupIntegrationTests, teardownIntegrationTests } = require('./database-cleanup');
const TilledClient = require('../../backend/src/tilledClient');

// Mock TilledClient
jest.mock('../../backend/src/tilledClient');

describe('Phase 4 Usage Routes Integration', () => {
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

  describe('POST /api/billing/usage/record', () => {
    it('records metered usage for a customer', async () => {
      // Create test customer
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_usage_001',
          tilled_customer_id: 'cus_test_usage',
          email: 'usage@example.com',
          name: 'Usage Test Customer'
        }
      });

      const response = await request(app)
        .post('/api/billing/usage/record')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: customer.id,
          metric_name: 'container_pickups',
          quantity: 10.5,
          unit_price_cents: 100, // $1.00 per pickup
          period_start: '2026-01-01T00:00:00Z',
          period_end: '2026-01-31T23:59:59Z'
        });

      expect(response.status).toBe(201);
      expect(response.body.usage_record).toBeDefined();
      expect(response.body.usage_record.customer_id).toBe(customer.id);
      expect(response.body.usage_record.metric_name).toBe('container_pickups');
      expect(response.body.usage_record.quantity).toBe(10.5);
      expect(response.body.usage_record.unit_price_cents).toBe(100);
      expect(response.body.usage_record.billed_at).toBeNull();

      // Verify record was created in database
      const usageRecord = await billingPrisma.billing_metered_usage.findFirst({
        where: { customer_id: customer.id }
      });
      expect(usageRecord).toBeDefined();
      expect(usageRecord.metric_name).toBe('container_pickups');
    });

    it('records metered usage with subscription ID', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_usage_sub_001',
          tilled_customer_id: 'cus_test_usage_sub',
          email: 'usage-sub@example.com',
          name: 'Usage Sub Test Customer'
        }
      });

      const subscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub_test_usage',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 15000,
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
        .post('/api/billing/usage/record')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: customer.id,
          subscription_id: subscription.id,
          metric_name: 'excess_weight',
          quantity: 25.5,
          unit_price_cents: 50, // $0.50 per kg
          period_start: '2026-01-15T00:00:00Z',
          period_end: '2026-01-31T23:59:59Z'
        });

      expect(response.status).toBe(201);
      expect(response.body.usage_record.subscription_id).toBe(subscription.id);
    });

    it('validates required parameters for record endpoint', async () => {
      const response = await request(app)
        .post('/api/billing/usage/record')
        .query({ app_id: 'trashtech' })
        .send({
          // Missing required fields
          customer_id: 999,
          metric_name: 'test'
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
    });

    it('returns 404 for non-existent customer', async () => {
      const response = await request(app)
        .post('/api/billing/usage/record')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: 999999,
          metric_name: 'container_pickups',
          quantity: 10,
          unit_price_cents: 100,
          period_start: '2026-01-01T00:00:00Z',
          period_end: '2026-01-31T23:59:59Z'
        });

      expect(response.status).toBe(404);
      expect(response.body.error).toContain('not found');
    });

    it('returns 404 for non-existent subscription', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_usage_badsub_001',
          tilled_customer_id: 'cus_test_badsub',
          email: 'badsub@example.com',
          name: 'Bad Sub Test Customer'
        }
      });

      const response = await request(app)
        .post('/api/billing/usage/record')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: customer.id,
          subscription_id: 999999,
          metric_name: 'container_pickups',
          quantity: 10,
          unit_price_cents: 100,
          period_start: '2026-01-01T00:00:00Z',
          period_end: '2026-01-31T23:59:59Z'
        });

      expect(response.status).toBe(404);
      expect(response.body.error).toContain('not found');
    });
  });

  describe('POST /api/billing/usage/calculate-charges', () => {
    it('calculates usage charges for billing period', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_calc_001',
          tilled_customer_id: 'cus_test_calc',
          email: 'calc@example.com',
          name: 'Calc Test Customer'
        }
      });

      // Create some usage records
      await billingPrisma.billing_metered_usage.create({
        data: {
          app_id: 'trashtech',
          customer_id: customer.id,
          metric_name: 'container_pickups',
          quantity: 10.0,
          unit_price_cents: 100,
          period_start: new Date('2026-01-10T00:00:00Z'),
          period_end: new Date('2026-01-20T23:59:59Z'),
          recorded_at: new Date('2026-01-20T10:00:00Z'),
          billed_at: null
        }
      });

      await billingPrisma.billing_metered_usage.create({
        data: {
          app_id: 'trashtech',
          customer_id: customer.id,
          metric_name: 'excess_weight',
          quantity: 50.0,
          unit_price_cents: 50,
          period_start: new Date('2026-01-05T00:00:00Z'),
          period_end: new Date('2026-01-15T23:59:59Z'),
          recorded_at: new Date('2026-01-15T10:00:00Z'),
          billed_at: null
        }
      });

      const response = await request(app)
        .post('/api/billing/usage/calculate-charges')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: customer.id,
          billing_period_start: '2026-01-01T00:00:00Z',
          billing_period_end: '2026-01-31T23:59:59Z',
          create_charges: false
        });

      expect(response.status).toBe(200);
      expect(response.body.usage_calculation).toBeDefined();
      expect(response.body.usage_calculation.customerId).toBe(customer.id);
      expect(response.body.usage_calculation.totalAmountCents).toBe(3500); // (10*100) + (50*50) = 1000 + 2500 = 3500
      expect(response.body.usage_calculation.metrics).toHaveLength(2);
      expect(response.body.usage_calculation.chargesCreated).toEqual([]);
    });

    it('calculates usage charges with subscription filter', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_calc_sub_001',
          tilled_customer_id: 'cus_test_calc_sub',
          email: 'calc-sub@example.com',
          name: 'Calc Sub Test Customer'
        }
      });

      const subscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub_test_calc',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 15000,
          status: 'active',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date('2026-01-01T00:00:00Z'),
          current_period_end: new Date('2026-02-01T00:00:00Z'),
          payment_method_id: 'pm_test',
          payment_method_type: 'card'
        }
      });

      // Create usage record with subscription
      await billingPrisma.billing_metered_usage.create({
        data: {
          app_id: 'trashtech',
          customer_id: customer.id,
          subscription_id: subscription.id,
          metric_name: 'container_pickups',
          quantity: 15.0,
          unit_price_cents: 100,
          period_start: new Date('2026-01-10T00:00:00Z'),
          period_end: new Date('2026-01-20T23:59:59Z'),
          recorded_at: new Date('2026-01-20T10:00:00Z'),
          billed_at: null
        }
      });

      const response = await request(app)
        .post('/api/billing/usage/calculate-charges')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: customer.id,
          subscription_id: subscription.id,
          billing_period_start: '2026-01-01T00:00:00Z',
          billing_period_end: '2026-01-31T23:59:59Z'
        });

      expect(response.status).toBe(200);
      expect(response.body.usage_calculation.subscriptionId).toBe(subscription.id);
      expect(response.body.usage_calculation.totalAmountCents).toBe(1500); // 15 * 100 = 1500
    });

    it('returns empty result when no unbilled usage found', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_calc_empty_001',
          tilled_customer_id: 'cus_test_calc_empty',
          email: 'calc-empty@example.com',
          name: 'Calc Empty Test Customer'
        }
      });

      const response = await request(app)
        .post('/api/billing/usage/calculate-charges')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: customer.id,
          billing_period_start: '2026-01-01T00:00:00Z',
          billing_period_end: '2026-01-31T23:59:59Z'
        });

      expect(response.status).toBe(200);
      expect(response.body.usage_calculation.totalAmountCents).toBe(0);
      expect(response.body.usage_calculation.summary).toContain('No unbilled usage found');
    });

    it('validates required parameters for calculate-charges endpoint', async () => {
      const response = await request(app)
        .post('/api/billing/usage/calculate-charges')
        .query({ app_id: 'trashtech' })
        .send({
          // Missing required fields
          customer_id: 999
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
    });

    it('returns 404 for non-existent customer', async () => {
      const response = await request(app)
        .post('/api/billing/usage/calculate-charges')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: 999999,
          billing_period_start: '2026-01-01T00:00:00Z',
          billing_period_end: '2026-01-31T23:59:59Z'
        });

      expect(response.status).toBe(404);
      expect(response.body.error).toContain('not found');
    });
  });

  describe('GET /api/billing/usage/report', () => {
    it('generates usage report with mixed billed/unbilled records', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_report_001',
          tilled_customer_id: 'cus_test_report',
          email: 'report@example.com',
          name: 'Report Test Customer'
        }
      });

      const subscription = await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub_test_report',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 15000,
          status: 'active',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date('2026-01-01T00:00:00Z'),
          current_period_end: new Date('2026-02-01T00:00:00Z'),
          payment_method_id: 'pm_test',
          payment_method_type: 'card'
        }
      });

      // Create billed usage record
      await billingPrisma.billing_metered_usage.create({
        data: {
          app_id: 'trashtech',
          customer_id: customer.id,
          subscription_id: subscription.id,
          metric_name: 'container_pickups',
          quantity: 10.0,
          unit_price_cents: 100,
          period_start: new Date('2026-01-10T00:00:00Z'),
          period_end: new Date('2026-01-20T23:59:59Z'),
          recorded_at: new Date('2026-01-20T10:00:00Z'),
          billed_at: new Date('2026-01-25T10:00:00Z')
        }
      });

      // Create unbilled usage record
      await billingPrisma.billing_metered_usage.create({
        data: {
          app_id: 'trashtech',
          customer_id: customer.id,
          subscription_id: subscription.id,
          metric_name: 'excess_weight',
          quantity: 25.5,
          unit_price_cents: 50,
          period_start: new Date('2026-01-15T00:00:00Z'),
          period_end: new Date('2026-01-31T23:59:59Z'),
          recorded_at: new Date('2026-01-31T10:00:00Z'),
          billed_at: null
        }
      });

      const response = await request(app)
        .get('/api/billing/usage/report')
        .query({
          app_id: 'trashtech',
          customer_id: customer.id,
          start_date: '2026-01-01T00:00:00Z',
          end_date: '2026-01-31T23:59:59Z',
          include_billed: true,
          include_unbilled: true
        });

      expect(response.status).toBe(200);
      expect(response.body.usage_report).toBeDefined();
      expect(response.body.usage_report.customerId).toBe(customer.id);
      expect(response.body.usage_report.summary.totalRecords).toBe(2);
      expect(response.body.usage_report.summary.totalAmountCents).toBe(2275); // (10*100) + (25.5*50) = 1000 + 1275 = 2275
      expect(response.body.usage_report.summary.billedAmountCents).toBe(1000);
      expect(response.body.usage_report.summary.unbilledAmountCents).toBe(1275);
      expect(response.body.usage_report.records).toHaveLength(2);

      // Verify subscription info is included
      const recordWithSubscription = response.body.usage_report.records.find(r => r.subscription !== null);
      expect(recordWithSubscription.subscription.plan_name).toBe('Pro Monthly');
    });

    it('filters report by billed/unbilled status', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_report_filter_001',
          tilled_customer_id: 'cus_test_report_filter',
          email: 'report-filter@example.com',
          name: 'Report Filter Test Customer'
        }
      });

      // Create billed record
      await billingPrisma.billing_metered_usage.create({
        data: {
          app_id: 'trashtech',
          customer_id: customer.id,
          metric_name: 'container_pickups',
          quantity: 10.0,
          unit_price_cents: 100,
          period_start: new Date('2026-01-10T00:00:00Z'),
          period_end: new Date('2026-01-20T23:59:59Z'),
          recorded_at: new Date('2026-01-20T10:00:00Z'),
          billed_at: new Date('2026-01-25T10:00:00Z')
        }
      });

      // Create unbilled record
      await billingPrisma.billing_metered_usage.create({
        data: {
          app_id: 'trashtech',
          customer_id: customer.id,
          metric_name: 'excess_weight',
          quantity: 25.0,
          unit_price_cents: 50,
          period_start: new Date('2026-01-15T00:00:00Z'),
          period_end: new Date('2026-01-31T23:59:59Z'),
          recorded_at: new Date('2026-01-31T10:00:00Z'),
          billed_at: null
        }
      });

      // Test billed only
      const billedResponse = await request(app)
        .get('/api/billing/usage/report')
        .query({
          app_id: 'trashtech',
          customer_id: customer.id,
          start_date: '2026-01-01T00:00:00Z',
          end_date: '2026-01-31T23:59:59Z',
          include_billed: true,
          include_unbilled: false
        });

      expect(billedResponse.status).toBe(200);
      expect(billedResponse.body.usage_report.summary.totalRecords).toBe(1);
      expect(billedResponse.body.usage_report.summary.totalAmountCents).toBe(1000);

      // Test unbilled only
      const unbilledResponse = await request(app)
        .get('/api/billing/usage/report')
        .query({
          app_id: 'trashtech',
          customer_id: customer.id,
          start_date: '2026-01-01T00:00:00Z',
          end_date: '2026-01-31T23:59:59Z',
          include_billed: false,
          include_unbilled: true
        });

      expect(unbilledResponse.status).toBe(200);
      expect(unbilledResponse.body.usage_report.summary.totalRecords).toBe(1);
      expect(unbilledResponse.body.usage_report.summary.totalAmountCents).toBe(1250);
    });

    it('generates empty report when no records match', async () => {
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_report_empty_001',
          tilled_customer_id: 'cus_test_report_empty',
          email: 'report-empty@example.com',
          name: 'Report Empty Test Customer'
        }
      });

      const response = await request(app)
        .get('/api/billing/usage/report')
        .query({
          app_id: 'trashtech',
          customer_id: customer.id,
          start_date: '2026-01-01T00:00:00Z',
          end_date: '2026-01-31T23:59:59Z'
        });

      expect(response.status).toBe(200);
      expect(response.body.usage_report.summary.totalRecords).toBe(0);
      expect(response.body.usage_report.summary.totalAmountCents).toBe(0);
    });

    it('validates required parameters for report endpoint', async () => {
      const response = await request(app)
        .get('/api/billing/usage/report')
        .query({
          app_id: 'trashtech',
          // Missing customer_id, start_date, end_date
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
    });

    it('returns 404 for non-existent customer', async () => {
      const response = await request(app)
        .get('/api/billing/usage/report')
        .query({
          app_id: 'trashtech',
          customer_id: 999999,
          start_date: '2026-01-01T00:00:00Z',
          end_date: '2026-01-31T23:59:59Z'
        });

      expect(response.status).toBe(404);
      expect(response.body.error).toContain('not found');
    });
  });
});