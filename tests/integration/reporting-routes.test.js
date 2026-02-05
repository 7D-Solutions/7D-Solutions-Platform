const express = require('express');
const request = require('supertest');
const { billingPrisma } = require('../../backend/src/prisma');
const { captureRawBody, rejectSensitiveData } = require('../../backend/src/middleware');
const { cleanDatabase, setupIntegrationTests, teardownIntegrationTests } = require('./database-cleanup');

// Mock Tilled client
jest.mock('../../backend/src/tilledClient');
const TilledClient = require('../../backend/src/tilledClient');

describe('Reporting Routes Integration Tests', () => {
  let app;
  const testAppId = 'test-app-reporting';

  // Mock Tilled client methods
  const mockTilledClient = {
    // Add minimal mock methods needed for reporting (likely none)
  };

  beforeAll(async () => {
    await setupIntegrationTests();

    // Setup mock
    TilledClient.mockImplementation(() => mockTilledClient);

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

  afterAll(async () => {
    await teardownIntegrationTests();
  });

  beforeEach(async () => {
    await cleanDatabase();
  });

  describe('GET /reports/revenue', () => {
    it('should return validation error for missing date range', async () => {
      const response = await request(app)
        .get('/api/billing/reports/revenue')
        .query({ app_id: testAppId });

      // With app_id but missing required query params
      expect(response.status).toBe(400);
      expect(response.body.error).toBeDefined();
    });

    it('should return 400 for invalid date range', async () => {
      const response = await request(app)
        .get('/api/billing/reports/revenue')
        .query({
          app_id: testAppId,
          start_date: '2026-01-31',
          end_date: '2026-01-01' // End before start
        });

      expect(response.status).toBe(400);
    });

    it('should return revenue report with test data', async () => {
      // Create test customer
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: testAppId,
          external_customer_id: 'cust-123',
          email: 'test@example.com',
          status: 'active'
        }
      });

      // Create test charges
      await billingPrisma.billing_charges.createMany({
        data: [
          {
            app_id: testAppId,
            billing_customer_id: customer.id,
            amount_cents: 5000,
            status: 'succeeded',
            charge_type: 'subscription',
            created_at: new Date('2026-01-15T10:00:00Z'),
            tilled_charge_id: 'ch_123',
            reference_id: 'ref-123'
          },
          {
            app_id: testAppId,
            billing_customer_id: customer.id,
            amount_cents: 3000,
            status: 'succeeded',
            charge_type: 'one_time',
            created_at: new Date('2026-01-16T14:00:00Z'),
            tilled_charge_id: 'ch_124',
            reference_id: 'ref-124'
          }
        ]
      });

      // Get the first charge ID for refund
      const firstCharge = await billingPrisma.billing_charges.findFirst({
        where: { app_id: testAppId, reference_id: 'ref-123' }
      });

      // Create test refund
      await billingPrisma.billing_refunds.create({
        data: {
          app_id: testAppId,
          billing_customer_id: customer.id,
          charge_id: firstCharge.id,
          amount_cents: 1000,
          status: 'succeeded',
          created_at: new Date('2026-01-17T16:00:00Z'),
          reference_id: 'refund-ref-123',
          currency: 'usd'
        }
      });

      const response = await request(app)
        .get('/api/billing/reports/revenue')
        .query({
          app_id: testAppId,
          start_date: '2026-01-01',
          end_date: '2026-01-31',
          granularity: 'daily'
        });

      if (response.status !== 200) {
        console.log('Error response:', response.status, response.body);
      }
      expect(response.status).toBe(200);
      expect(response.body.revenue_report).toBeDefined();
      expect(response.body.revenue_report.summary.total_gross_revenue_cents).toBe(8000);
      expect(response.body.revenue_report.summary.total_refunds_cents).toBe(1000);
      expect(response.body.revenue_report.summary.total_net_revenue_cents).toBe(7000);
      expect(response.body.revenue_report.periods.length).toBeGreaterThan(0);
    });

    it('should filter by customer_id', async () => {
      // Create two customers
      const customer1 = await billingPrisma.billing_customers.create({
        data: {
          app_id: testAppId,
          external_customer_id: 'cust-1',
          email: 'test1@example.com',
          status: 'active'
        }
      });

      const customer2 = await billingPrisma.billing_customers.create({
        data: {
          app_id: testAppId,
          external_customer_id: 'cust-2',
          email: 'test2@example.com',
          status: 'active'
        }
      });

      // Create charges for both customers
      await billingPrisma.billing_charges.createMany({
        data: [
          {
            app_id: testAppId,
            billing_customer_id: customer1.id,
            amount_cents: 5000,
            status: 'succeeded',
            charge_type: 'subscription',
            created_at: new Date('2026-01-15T10:00:00Z'),
            tilled_charge_id: 'ch_cust1',
            reference_id: 'ref-cust1'
          },
          {
            app_id: testAppId,
            billing_customer_id: customer2.id,
            amount_cents: 3000,
            status: 'succeeded',
            charge_type: 'one_time',
            created_at: new Date('2026-01-16T14:00:00Z'),
            tilled_charge_id: 'ch_cust2',
            reference_id: 'ref-cust2'
          }
        ]
      });

      const response = await request(app)
        .get('/api/billing/reports/revenue')
        .query({
          app_id: testAppId,
          start_date: '2026-01-01',
          end_date: '2026-01-31',
          customer_id: customer1.id
        });

      expect(response.status).toBe(200);
      expect(response.body.revenue_report.summary.total_gross_revenue_cents).toBe(5000);
    });
  });

  describe('GET /reports/mrr', () => {
    it('should return 400 for missing as_of_date', async () => {
      const response = await request(app)
        .get('/api/billing/reports/mrr')
        .query({ app_id: testAppId });

      expect(response.status).toBe(400);
    });

    it('should return MRR report with active subscriptions', async () => {
      // Create test customer
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: testAppId,
          external_customer_id: 'cust-mrr',
          email: 'mrr@example.com',
          status: 'active'
        }
      });

      // Create active subscriptions with different intervals
      await billingPrisma.billing_subscriptions.createMany({
        data: [
          {
            app_id: testAppId,
            billing_customer_id: customer.id,
            tilled_subscription_id: 'sub-monthly-1',
            plan_id: 'pro-monthly',
            plan_name: 'Pro Monthly',
            price_cents: 10000,
            status: 'active',
            interval_unit: 'month',
            interval_count: 1,
            current_period_start: new Date('2026-01-01T00:00:00Z'),
            current_period_end: new Date('2026-02-01T00:00:00Z'),
            payment_method_id: 'pm-123',
            payment_method_type: 'card'
          },
          {
            app_id: testAppId,
            billing_customer_id: customer.id,
            tilled_subscription_id: 'sub-annual-1',
            plan_id: 'pro-annual',
            plan_name: 'Pro Annual',
            price_cents: 120000, // $1200/year = $100/month MRR
            status: 'active',
            interval_unit: 'year',
            interval_count: 1,
            current_period_start: new Date('2026-01-01T00:00:00Z'),
            current_period_end: new Date('2027-01-01T00:00:00Z'),
            payment_method_id: 'pm-123',
            payment_method_type: 'card'
          }
        ]
      });

      const response = await request(app)
        .get('/api/billing/reports/mrr')
        .query({
          app_id: testAppId,
          as_of_date: '2026-01-15' // Mid-period for both subscriptions
        });

      expect(response.status).toBe(200);
      expect(response.body.mrr_report).toBeDefined();
      expect(response.body.mrr_report.total_mrr_cents).toBe(20000); // $100 + $100 = $200/month
      expect(response.body.mrr_report.subscription_count).toBe(2);
      expect(response.body.mrr_report.breakdown).toHaveLength(2);
    });

    it('should exclude subscriptions outside current period', async () => {
      // Create test customer
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: testAppId,
          external_customer_id: 'cust-mrr-2',
          email: 'mrr2@example.com',
          status: 'active'
        }
      });

      // Create subscription that ended before as_of_date
      await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: testAppId,
          billing_customer_id: customer.id,
          tilled_subscription_id: 'sub-expired',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 10000,
          status: 'canceled',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date('2025-12-01T00:00:00Z'),
          current_period_end: new Date('2026-01-01T00:00:00Z'), // Ended before as_of_date
          canceled_at: new Date('2026-01-01T00:00:00Z'),
          payment_method_id: 'pm-123',
          payment_method_type: 'card'
        }
      });

      const response = await request(app)
        .get('/api/billing/reports/mrr')
        .query({
          app_id: testAppId,
          as_of_date: '2026-01-15'
        });

      expect(response.status).toBe(200);
      expect(response.body.mrr_report.total_mrr_cents).toBe(0);
      expect(response.body.mrr_report.subscription_count).toBe(0);
    });
  });

  describe('GET /reports/churn', () => {
    it('should return 400 for missing date range', async () => {
      const response = await request(app)
        .get('/api/billing/reports/churn')
        .query({ app_id: testAppId });

      expect(response.status).toBe(400);
    });

    it('should return churn report with canceled subscriptions', async () => {
      // Create test customers
      const customer1 = await billingPrisma.billing_customers.create({
        data: {
          app_id: testAppId,
          external_customer_id: 'cust-churn-1',
          email: 'churn1@example.com',
          status: 'active'
        }
      });

      const customer2 = await billingPrisma.billing_customers.create({
        data: {
          app_id: testAppId,
          external_customer_id: 'cust-churn-2',
          email: 'churn2@example.com',
          status: 'active'
        }
      });

      // Create active subscription for customer1 (starting active)
      await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: testAppId,
          billing_customer_id: customer1.id,
          tilled_subscription_id: 'sub-active-1',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 10000,
          status: 'active',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date('2025-12-01T00:00:00Z'),
          current_period_end: new Date('2026-01-01T00:00:00Z'),
          payment_method_id: 'pm-123',
          payment_method_type: 'card',
          created_at: new Date('2025-12-01T00:00:00Z')
        }
      });

      // Create canceled subscription in churn period
      await billingPrisma.billing_subscriptions.create({
        data: {
          app_id: testAppId,
          billing_customer_id: customer2.id,
          tilled_subscription_id: 'sub-canceled-1',
          plan_id: 'pro-monthly',
          plan_name: 'Pro Monthly',
          price_cents: 10000,
          status: 'canceled',
          interval_unit: 'month',
          interval_count: 1,
          current_period_start: new Date('2025-12-01T00:00:00Z'),
          current_period_end: new Date('2026-01-01T00:00:00Z'),
          canceled_at: new Date('2026-01-15T10:00:00Z'), // Canceled in churn period
          payment_method_id: 'pm-123',
          payment_method_type: 'card',
          created_at: new Date('2025-12-01T00:00:00Z')
        }
      });

      const response = await request(app)
        .get('/api/billing/reports/churn')
        .query({
          app_id: testAppId,
          start_date: '2026-01-01',
          end_date: '2026-01-31',
          cohort_period: 'monthly'
        });

      expect(response.status).toBe(200);
      expect(response.body.churn_report).toBeDefined();
      expect(response.body.churn_report.overall.starting_customer_count).toBe(1); // customer1
      expect(response.body.churn_report.overall.churned_customer_count).toBe(1); // customer2
      expect(response.body.churn_report.overall.customer_churn_rate).toBe(1.0); // 1/1 = 100% churn
    });
  });

  describe('GET /reports/aging-receivables', () => {
    it('should return 400 for missing as_of_date', async () => {
      const response = await request(app)
        .get('/api/billing/reports/aging-receivables')
        .query({ app_id: testAppId });

      expect(response.status).toBe(400);
    });

    it('should return aging receivables report', async () => {
      // Create test customer
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: testAppId,
          external_customer_id: 'cust-aging',
          email: 'aging@example.com',
          status: 'active'
        }
      });

      // Create invoices with different due dates
      await billingPrisma.billing_invoices.createMany({
        data: [
          {
            app_id: testAppId,
            billing_customer_id: customer.id,
            tilled_invoice_id: 'inv-1',
            amount_cents: 10000,
            status: 'open',
            due_at: new Date('2026-01-31T00:00:00Z'), // Current (due on as_of_date)
            currency: 'usd'
          },
          {
            app_id: testAppId,
            billing_customer_id: customer.id,
            tilled_invoice_id: 'inv-2',
            amount_cents: 5000,
            status: 'past_due',
            due_at: new Date('2026-01-01T00:00:00Z'), // 30 days overdue
            currency: 'usd'
          },
          {
            app_id: testAppId,
            billing_customer_id: customer.id,
            tilled_invoice_id: 'inv-3',
            amount_cents: 3000,
            status: 'paid', // Fully paid
            due_at: new Date('2026-01-01T00:00:00Z'),
            paid_at: new Date('2026-01-01T00:00:00Z'),
            currency: 'usd'
          }
        ]
      });

      const response = await request(app)
        .get('/api/billing/reports/aging-receivables')
        .query({
          app_id: testAppId,
          as_of_date: '2026-01-31'
        });

      if (response.status !== 200) {
        console.log('Aging receivables error:', response.status, response.body);
      }
      expect(response.status).toBe(200);
      expect(response.body.aging_receivables_report).toBeDefined();
      expect(response.body.aging_receivables_report.total_outstanding_cents).toBe(15000); // 10000 + 5000
      expect(response.body.aging_receivables_report.total_invoice_count).toBe(2); // Only open + past_due (paid excluded)
      expect(response.body.aging_receivables_report.aging_buckets.length).toBeGreaterThan(0);

      // Verify buckets exist
      const buckets = response.body.aging_receivables_report.aging_buckets;
      expect(buckets.some(b => b.bucket === 'current')).toBe(true);
      expect(buckets.some(b => b.bucket === '1-30')).toBe(true);
    });
  });

  describe('Authentication and Authorization', () => {
    it('should return 400 without app_id parameter', async () => {
      const response = await request(app)
        .get('/api/billing/reports/revenue')
        .query({
          start_date: '2026-01-01',
          end_date: '2026-01-31'
        });

      // requireAppId middleware returns 400 for missing app_id
      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Missing app_id');
    });

    it('should respect app_id isolation', async () => {
      // Create data for testAppId
      const customer = await billingPrisma.billing_customers.create({
        data: {
          app_id: testAppId,
          external_customer_id: 'cust-isolated',
          email: 'isolated@example.com',
          status: 'active'
        }
      });

      await billingPrisma.billing_charges.create({
        data: {
          app_id: testAppId,
          billing_customer_id: customer.id,
          amount_cents: 5000,
          status: 'succeeded',
          charge_type: 'subscription',
          created_at: new Date('2026-01-15T10:00:00Z'),
          tilled_charge_id: 'ch_iso',
          reference_id: 'ref-iso'
        }
      });

      // Query with different app_id
      const response = await request(app)
        .get('/api/billing/reports/revenue')
        .query({
          app_id: 'different-app',
          start_date: '2026-01-01',
          end_date: '2026-01-31'
        });

      expect(response.status).toBe(200);
      // Should not see testAppId's data
      expect(response.body.revenue_report.summary.total_gross_revenue_cents).toBe(0);
    });
  });
});