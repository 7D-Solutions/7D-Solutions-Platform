/**
 * Integration tests for Phase 5 Invoice Routes
 *
 * @integration - Uses real database, mocked Tilled API
 * Tests full request/response cycle through Express routes
 */

const request = require('supertest');
const express = require('express');
const routes = require('../../backend/src/routes/index');
const handleBillingError = require('../../backend/src/middleware/errorHandler');
const { billingPrisma } = require('../../backend/src/prisma');
const { cleanDatabase, setupIntegrationTests, teardownIntegrationTests } = require('./database-cleanup');
const TilledClient = require('../../backend/src/tilledClient');

// Mock TilledClient
jest.mock('../../backend/src/tilledClient');

describe('Phase 5 Invoice Routes Integration', () => {
  let app;
  let mockTilledClient;
  let testCustomer;
  let testSubscription;

  beforeAll(async () => {
    await setupIntegrationTests();

    app = express();
    app.use(express.json());
    app.use('/api/billing', routes);
    app.use(handleBillingError); // Error handler MUST be mounted last

    mockTilledClient = {
      createCharge: jest.fn(),
    };
    TilledClient.mockImplementation(() => mockTilledClient);
  });

  beforeEach(async () => {
    // Clean up database using centralized cleanup (wrapped in transaction)
    await cleanDatabase();
    jest.clearAllMocks();

    // Create test customer for all tests
    testCustomer = await billingPrisma.billing_customers.create({
      data: {
        app_id: 'trashtech',
        external_customer_id: 'ext_invoice_test_001',
        tilled_customer_id: 'cus_invoice_test',
        email: 'invoice@example.com',
        name: 'Invoice Test Customer',
        default_payment_method_id: 'pm_test_invoice',
        payment_method_type: 'card',
      },
    });

    // Create test subscription
    testSubscription = await billingPrisma.billing_subscriptions.create({
      data: {
        app_id: 'trashtech',
        billing_customer_id: testCustomer.id,
        tilled_subscription_id: 'sub_invoice_test',
        plan_id: 'pro-monthly',
        plan_name: 'Pro Monthly',
        price_cents: 10000, // $100
        status: 'active',
        interval_unit: 'month',
        interval_count: 1,
        current_period_start: new Date('2026-01-01T00:00:00Z'),
        current_period_end: new Date('2026-02-01T00:00:00Z'),
        payment_method_id: 'pm_test_invoice',
        payment_method_type: 'card',
      },
    });
  });

  afterAll(async () => {
    await teardownIntegrationTests();
  });

  describe('POST /api/billing/invoices', () => {
    it('should create invoice successfully', async () => {
      const response = await request(app)
        .post('/api/billing/invoices')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: testCustomer.id,
          subscription_id: testSubscription.id,
          amount_cents: 10000,
          due_date: '2026-02-15T00:00:00Z',
          description: 'Monthly subscription invoice',
        });

      expect(response.status).toBe(201);
      expect(response.body).toMatchObject({
        billing_customer_id: testCustomer.id,
        subscription_id: testSubscription.id,
        amount_cents: 10000,
        status: 'draft',
      });
      expect(response.body).toHaveProperty('id');
      expect(response.body).toHaveProperty('tilled_invoice_id');

      // Verify invoice was created in database
      const invoice = await billingPrisma.billing_invoices.findUnique({
        where: { id: response.body.id },
      });
      expect(invoice).toBeTruthy();
      expect(invoice.amount_cents).toBe(10000);
    });

    it('should return 400 for missing required fields', async () => {
      const response = await request(app)
        .post('/api/billing/invoices')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: testCustomer.id,
          // Missing amount_cents
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
    });

    it('should return 400 for negative amount', async () => {
      const response = await request(app)
        .post('/api/billing/invoices')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: testCustomer.id,
          amount_cents: -1000,
        });

      expect(response.status).toBe(400);
    });

    it('should return 404 for non-existent customer', async () => {
      const response = await request(app)
        .post('/api/billing/invoices')
        .query({ app_id: 'trashtech' })
        .send({
          customer_id: 99999,
          amount_cents: 10000,
        });

      expect(response.status).toBe(404);
    });
  });

  describe('GET /api/billing/invoices/:id', () => {
    let testInvoice;

    beforeEach(async () => {
      // Create test invoice
      testInvoice = await billingPrisma.billing_invoices.create({
        data: {
          app_id: 'trashtech',
          tilled_invoice_id: 'in_test_001',
          billing_customer_id: testCustomer.id,
          subscription_id: testSubscription.id,
          status: 'draft',
          amount_cents: 10000,
          due_at: new Date('2026-02-15T00:00:00Z'),
        },
      });
    });

    it('should get invoice by id', async () => {
      const response = await request(app)
        .get(`/api/billing/invoices/${testInvoice.id}`)
        .query({ app_id: 'trashtech' });

      expect(response.status).toBe(200);
      expect(response.body).toMatchObject({
        id: testInvoice.id,
        tilled_invoice_id: 'in_test_001',
        amount_cents: 10000,
        status: 'draft',
      });
    });

    it('should return 404 for non-existent invoice', async () => {
      const response = await request(app)
        .get('/api/billing/invoices/99999')
        .query({ app_id: 'trashtech' });

      expect(response.status).toBe(404);
    });

    it('should return 400 when app_id missing', async () => {
      const response = await request(app)
        .get(`/api/billing/invoices/${testInvoice.id}`);

      expect(response.status).toBe(400);
    });
  });

  describe('POST /api/billing/invoices/:id/line-items', () => {
    let testInvoice;

    beforeEach(async () => {
      testInvoice = await billingPrisma.billing_invoices.create({
        data: {
          app_id: 'trashtech',
          tilled_invoice_id: 'in_test_002',
          billing_customer_id: testCustomer.id,
          subscription_id: testSubscription.id,
          amount_cents: 0,
          status: 'draft',
          due_at: new Date('2026-02-15T00:00:00Z'),
        },
      });
    });

    it('should add line item to invoice', async () => {
      const response = await request(app)
        .post(`/api/billing/invoices/${testInvoice.id}/line-items`)
        .query({ app_id: 'trashtech' })
        .send({
          line_item_type: 'subscription',
          description: 'Pro Monthly Subscription',
          quantity: 1,
          unit_price_cents: 10000,
          amount_cents: 10000,
        });

      expect(response.status).toBe(201);
      expect(response.body).toMatchObject({
        invoice_id: testInvoice.id,
        description: 'Pro Monthly Subscription',
        quantity: 1,
        unit_price_cents: 10000,
        amount_cents: 10000,
      });

      // Verify invoice amount was updated by fetching from database
      const updatedInvoice = await billingPrisma.billing_invoices.findUnique({
        where: { id: testInvoice.id },
      });
      expect(updatedInvoice.amount_cents).toBe(10000);

      // Verify line item was created in database
      const lineItems = await billingPrisma.billing_invoice_line_items.findMany({
        where: { invoice_id: testInvoice.id },
      });
      expect(lineItems).toHaveLength(1);
      expect(lineItems[0].amount_cents).toBe(10000);
    });

    it('should add multiple line items and sum amounts', async () => {
      // Add first line item
      await request(app)
        .post(`/api/billing/invoices/${testInvoice.id}/line-items`)
        .query({ app_id: 'trashtech' })
        .send({
          line_item_type: 'subscription',
          description: 'Base subscription',
          quantity: 1,
          unit_price_cents: 10000,
          amount_cents: 10000,
        });

      // Add second line item
      const response = await request(app)
        .post(`/api/billing/invoices/${testInvoice.id}/line-items`)
        .query({ app_id: 'trashtech' })
        .send({
          line_item_type: 'fee',
          description: 'Add-on service',
          quantity: 2,
          unit_price_cents: 2500,
          amount_cents: 5000,
        });

      expect(response.status).toBe(201);
      // Verify invoice amount was updated
      const updatedInvoice = await billingPrisma.billing_invoices.findUnique({
        where: { id: testInvoice.id },
      });
      expect(updatedInvoice.amount_cents).toBe(15000);

      // Verify both line items exist
      const lineItems = await billingPrisma.billing_invoice_line_items.findMany({
        where: { invoice_id: testInvoice.id },
      });
      expect(lineItems).toHaveLength(2);
    });

    it('should return 400 for missing required fields', async () => {
      const response = await request(app)
        .post(`/api/billing/invoices/${testInvoice.id}/line-items`)
        .query({ app_id: 'trashtech' })
        .send({
          description: 'Incomplete item',
          // Missing quantity and amount_cents
        });

      expect(response.status).toBe(400);
    });

    it('should return 404 for non-existent invoice', async () => {
      const response = await request(app)
        .post('/api/billing/invoices/99999/line-items')
        .query({ app_id: 'trashtech' })
        .send({
          line_item_type: 'fee',
          description: 'Test item',
          quantity: 1,
          unit_price_cents: 1000,
          amount_cents: 1000,
        });

      expect(response.status).toBe(404);
    });
  });

  describe('POST /api/billing/invoices/generate-from-subscription', () => {
    it('should generate invoice from subscription', async () => {
      const response = await request(app)
        .post('/api/billing/invoices/generate-from-subscription')
        .query({ app_id: 'trashtech' })
        .send({
          subscription_id: testSubscription.id,
          billing_period_start: '2026-01-01T00:00:00Z',
          billing_period_end: '2026-02-01T00:00:00Z',
        });

      expect(response.status).toBe(201);
      expect(response.body).toMatchObject({
        billing_customer_id: testCustomer.id,
        subscription_id: testSubscription.id,
        amount_cents: 10000, // subscription price
        status: 'draft',
      });
      expect(response.body).toHaveProperty('tilled_invoice_id');

      // Verify invoice was created
      const invoice = await billingPrisma.billing_invoices.findUnique({
        where: { id: response.body.id },
      });
      expect(invoice).toBeTruthy();
    });

    it('should return 404 for non-existent subscription', async () => {
      const response = await request(app)
        .post('/api/billing/invoices/generate-from-subscription')
        .query({ app_id: 'trashtech' })
        .send({
          subscription_id: 99999,
          billing_period_start: '2026-01-01T00:00:00Z',
          billing_period_end: '2026-02-01T00:00:00Z',
        });

      expect(response.status).toBe(404);
    });

    it('should return 400 for missing required fields', async () => {
      const response = await request(app)
        .post('/api/billing/invoices/generate-from-subscription')
        .query({ app_id: 'trashtech' })
        .send({
          subscription_id: testSubscription.id,
          // Missing period dates
        });

      expect(response.status).toBe(400);
    });
  });

  describe('PATCH /api/billing/invoices/:id/status', () => {
    let testInvoice;

    beforeEach(async () => {
      testInvoice = await billingPrisma.billing_invoices.create({
        data: {
          app_id: 'trashtech',
          tilled_invoice_id: 'in_test_003',
          billing_customer_id: testCustomer.id,
          subscription_id: testSubscription.id,
          amount_cents: 10000,
          status: 'draft',
          due_at: new Date('2026-02-15T00:00:00Z'),
        },
      });
    });

    it('should update invoice status to sent', async () => {
      const response = await request(app)
        .patch(`/api/billing/invoices/${testInvoice.id}/status`)
        .query({ app_id: 'trashtech' })
        .send({
          status: 'open',
        });

      expect(response.status).toBe(200);
      expect(response.body.status).toBe('open');

      // Verify status was updated in database
      const invoice = await billingPrisma.billing_invoices.findUnique({
        where: { id: testInvoice.id },
      });
      expect(invoice.status).toBe('open');
    });

    it('should update invoice status to paid', async () => {
      const response = await request(app)
        .patch(`/api/billing/invoices/${testInvoice.id}/status`)
        .query({ app_id: 'trashtech' })
        .send({
          status: 'paid',
        });

      expect(response.status).toBe(200);
      expect(response.body.status).toBe('paid');
    });

    it('should update invoice status to void', async () => {
      const response = await request(app)
        .patch(`/api/billing/invoices/${testInvoice.id}/status`)
        .query({ app_id: 'trashtech' })
        .send({
          status: 'void',
        });

      expect(response.status).toBe(200);
      expect(response.body.status).toBe('void');
    });

    it('should return 400 for invalid status', async () => {
      const response = await request(app)
        .patch(`/api/billing/invoices/${testInvoice.id}/status`)
        .query({ app_id: 'trashtech' })
        .send({
          status: 'invalid_status',
        });

      expect(response.status).toBe(400);
    });

    it('should return 404 for non-existent invoice', async () => {
      const response = await request(app)
        .patch('/api/billing/invoices/99999/status')
        .query({ app_id: 'trashtech' })
        .send({
          status: 'open',
        });

      expect(response.status).toBe(404);
    });
  });

  describe('GET /api/billing/invoices', () => {
    beforeEach(async () => {
      // Create multiple test invoices
      await billingPrisma.billing_invoices.createMany({
        data: [
          {
            app_id: 'trashtech',
            tilled_invoice_id: 'in_test_101',
            billing_customer_id: testCustomer.id,
            subscription_id: testSubscription.id,
            amount_cents: 10000,
            status: 'draft',
            due_at: new Date('2026-02-15T00:00:00Z'),
          },
          {
            app_id: 'trashtech',
            tilled_invoice_id: 'in_test_102',
            billing_customer_id: testCustomer.id,
            subscription_id: testSubscription.id,
            amount_cents: 15000,
            status: 'sent',
            due_at: new Date('2026-02-20T00:00:00Z'),
          },
          {
            app_id: 'trashtech',
            tilled_invoice_id: 'in_test_103',
            billing_customer_id: testCustomer.id,
            subscription_id: testSubscription.id,
            amount_cents: 20000,
            status: 'paid',
            due_at: new Date('2026-02-25T00:00:00Z'),
          },
        ],
      });
    });

    it('should list all invoices for customer', async () => {
      const response = await request(app)
        .get('/api/billing/invoices')
        .query({
          app_id: 'trashtech',
          customer_id: testCustomer.id,
        });

      expect(response.status).toBe(200);
      expect(response.body.invoices).toHaveLength(3);
      expect(response.body.pagination).toMatchObject({
        total: 3,
        hasMore: false,
      });
      expect(response.body.invoices[0]).toHaveProperty('tilled_invoice_id');
      expect(response.body.invoices[0]).toHaveProperty('amount_cents');
      expect(response.body.invoices[0]).toHaveProperty('status');
    });

    it('should filter invoices by status', async () => {
      const response = await request(app)
        .get('/api/billing/invoices')
        .query({
          app_id: 'trashtech',
          customer_id: testCustomer.id,
          status: 'paid',
        });

      expect(response.status).toBe(200);
      expect(response.body.invoices).toHaveLength(1);
      expect(response.body.invoices[0].status).toBe('paid');
      expect(response.body.invoices[0].tilled_invoice_id).toBe('in_test_103');
    });

    it('should filter invoices by subscription', async () => {
      const response = await request(app)
        .get('/api/billing/invoices')
        .query({
          app_id: 'trashtech',
          subscription_id: testSubscription.id,
        });

      expect(response.status).toBe(200);
      expect(response.body.invoices).toHaveLength(3);
    });

    it('should return empty array when no invoices found', async () => {
      const otherCustomer = await billingPrisma.billing_customers.create({
        data: {
          app_id: 'trashtech',
          external_customer_id: 'ext_other_customer',
          tilled_customer_id: 'cus_other',
          email: 'other@example.com',
          name: 'Other Customer',
        },
      });

      const response = await request(app)
        .get('/api/billing/invoices')
        .query({
          app_id: 'trashtech',
          customer_id: otherCustomer.id,
        });

      expect(response.status).toBe(200);
      expect(response.body.invoices).toHaveLength(0);
      expect(response.body.pagination.total).toBe(0);
    });

    it('should return 400 when app_id missing', async () => {
      const response = await request(app)
        .get('/api/billing/invoices')
        .query({ customer_id: testCustomer.id });

      expect(response.status).toBe(400);
    });
  });
});
