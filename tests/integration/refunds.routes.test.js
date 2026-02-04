const request = require('supertest');
const express = require('express');
const routes = require('../../backend/src/routes');
const handleBillingError = require('../../backend/src/middleware/errorHandler');
const { billingPrisma } = require('../../backend/src/prisma');
const { cleanDatabase, setupIntegrationTests, teardownIntegrationTests } = require('./database-cleanup');
const TilledClient = require('../../backend/src/tilledClient');

// Mock TilledClient
jest.mock('../../backend/src/tilledClient');

describe('POST /api/billing/refunds Integration Tests', () => {
  let app;
  let mockTilledClient;
  let testCustomer;
  let testCharge;

  beforeAll(async () => {
    await setupIntegrationTests();

    app = express();
    app.use(express.json());
    app.use('/api/billing', routes);
    app.use(handleBillingError); // Error handler MUST be mounted last

    mockTilledClient = {
      createRefund: jest.fn(),
    };
    TilledClient.mockImplementation(() => mockTilledClient);
  });

  beforeEach(async () => {
    // Clean up database using centralized cleanup (wrapped in transaction)
    await cleanDatabase();
    jest.clearAllMocks();

    // Create test customer
    testCustomer = await billingPrisma.billing_customers.create({
      data: {
        app_id: 'trashtech',
        external_customer_id: 'ext_test_123',
        tilled_customer_id: 'cus_test_123',
        email: 'test@example.com',
        name: 'Test Customer',
        default_payment_method_id: 'pm_test_123',
        payment_method_type: 'card',
      },
    });

    // Create test charge
    testCharge = await billingPrisma.billing_charges.create({
      data: {
        app_id: 'trashtech',
        billing_customer_id: testCustomer.id,
        tilled_charge_id: 'ch_test_123',
        status: 'succeeded',
        amount_cents: 5000,
        currency: 'usd',
        charge_type: 'one_time',
        reason: 'extra_pickup',
        reference_id: 'charge_ref_123',
      },
    });
  });

  afterAll(async () => {
    await teardownIntegrationTests();
  });

  describe('Validation Tests', () => {
    it('returns 400 if app_id is missing', async () => {
      const response = await request(app)
        .post('/api/billing/refunds') // No app_id in query
        .set('Idempotency-Key', 'test-key-1')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_test_1',
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toContain('app_id');
    });

    it('returns 400 if Idempotency-Key header is missing', async () => {
      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_test_2',
        });

      expect(response.status).toBe(400);
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

    it('returns 400 if charge_id is missing', async () => {
      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-3')
        .send({
          amount_cents: 1000,
          reference_id: 'refund_test_3',
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
      expect(response.body.details).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            field: 'charge_id',
            message: expect.stringContaining('charge_id')
          })
        ])
      );
    });

    it('returns 400 if amount_cents is missing', async () => {
      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-4')
        .send({
          charge_id: testCharge.id,
          reference_id: 'refund_test_4',
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
      expect(response.body.details).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            field: 'amount_cents',
            message: expect.stringContaining('amount_cents')
          })
        ])
      );
    });

    it('returns 400 if amount_cents is less than or equal to 0', async () => {
      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-5')
        .send({
          charge_id: testCharge.id,
          amount_cents: 0,
          reference_id: 'refund_test_5',
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
      expect(response.body.details).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            field: 'amount_cents',
            message: expect.stringContaining('positive integer')
          })
        ])
      );
    });

    it('returns 400 if reference_id is missing', async () => {
      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-6')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
      expect(response.body.details).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            field: 'reference_id',
            message: expect.stringContaining('reference_id')
          })
        ])
      );
    });

    it('returns 400 if reference_id is empty string', async () => {
      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-7')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: '',
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
      expect(response.body.details).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            field: 'reference_id',
            message: expect.stringContaining('reference_id')
          })
        ])
      );
    });

    it('returns 400 if reference_id is whitespace only', async () => {
      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-8')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: '   ',
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Validation failed');
      expect(response.body.details).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            field: 'reference_id',
            message: expect.stringContaining('reference_id')
          })
        ])
      );
    });

    it('returns 400 if body contains PCI-sensitive data (card_number)', async () => {
      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-9')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_test_9',
          card_number: '4242424242424242', // PCI violation
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toContain('PCI violation');
    });

    it('returns 400 if body contains PCI-sensitive data (cvv)', async () => {
      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-10')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_test_10',
          cvv: '123', // PCI violation
        });

      expect(response.status).toBe(400);
      expect(response.body.error).toContain('PCI violation');
    });
  });

  describe('Authorization Tests', () => {
    it('returns 404 if charge_id not found', async () => {
      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-11')
        .send({
          charge_id: 999999,
          amount_cents: 1000,
          reference_id: 'refund_test_11',
        });

      expect(response.status).toBe(404);
      expect(response.body.error).toContain('Charge not found');
    });

    it('returns 404 if charge exists but belongs to different app_id (no ID leakage)', async () => {
      // Create charge for different app
      const otherAppCharge = await billingPrisma.billing_charges.create({
        data: {
          app_id: 'otherapp',
          billing_customer_id: testCustomer.id,
          tilled_charge_id: 'ch_other_123',
          status: 'succeeded',
          amount_cents: 5000,
          currency: 'usd',
          charge_type: 'one_time',
          reason: 'test',
          reference_id: 'other_charge_ref',
        },
      });

      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech') // Different app_id
        .set('Idempotency-Key', 'test-key-12')
        .send({
          charge_id: otherAppCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_test_12',
        });

      expect(response.status).toBe(404);
      expect(response.body.error).toContain('Charge not found');

      // Verify no refund was created
      const refundCount = await billingPrisma.billing_refunds.count();
      expect(refundCount).toBe(0);
    });

    it('returns 409 if charge has no tilled_charge_id (not settled in processor)', async () => {
      // Create charge without tilled_charge_id
      const unsettledCharge = await billingPrisma.billing_charges.create({
        data: {
          app_id: 'trashtech',
          billing_customer_id: testCustomer.id,
          tilled_charge_id: null, // Not settled
          status: 'pending',
          amount_cents: 5000,
          currency: 'usd',
          charge_type: 'one_time',
          reason: 'test',
          reference_id: 'unsettled_charge_ref',
        },
      });

      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-13')
        .send({
          charge_id: unsettledCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_test_13',
        });

      expect(response.status).toBe(409);
      expect(response.body.error).toMatch(/not settled|processor/i);
    });
  });

  describe('Success Path Tests', () => {
    it('returns 201 and creates refund successfully', async () => {
      mockTilledClient.createRefund.mockResolvedValue({
        id: 'rf_tilled_123',
        status: 'succeeded',
        amount: 1000,
        currency: 'usd',
      });

      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-14')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          currency: 'usd',
          reason: 'requested_by_customer',
          reference_id: 'refund_test_14',
          note: 'Customer requested refund',
          metadata: { ticket_id: 'T123' },
        });

      expect(response.status).toBe(201);
      expect(response.body.refund).toBeDefined();
      expect(response.body.refund.tilled_refund_id).toBe('rf_tilled_123');
      expect(response.body.refund.status).toBe('succeeded');
      expect(response.body.refund.amount_cents).toBe(1000);
      expect(response.body.refund.reference_id).toBe('refund_test_14');

      // Verify database record
      const refund = await billingPrisma.billing_refunds.findFirst({
        where: { reference_id: 'refund_test_14' },
      });
      expect(refund).not.toBeNull();
      expect(refund.app_id).toBe('trashtech');
      expect(refund.charge_id).toBe(testCharge.id);
      expect(refund.billing_customer_id).toBe(testCustomer.id);
      expect(refund.tilled_refund_id).toBe('rf_tilled_123');
      expect(refund.status).toBe('succeeded');
      expect(refund.note).toBe('Customer requested refund');
      expect(refund.metadata.ticket_id).toBe('T123');

      // Verify Tilled was called correctly
      expect(mockTilledClient.createRefund).toHaveBeenCalledWith({
        appId: 'trashtech',
        tilledChargeId: 'ch_test_123',
        amountCents: 1000,
        currency: 'usd',
        reason: 'requested_by_customer',
        metadata: expect.objectContaining({
          ticket_id: 'T123',
          reference_id: 'refund_test_14',
        }),
      });
    });
  });

  describe('Idempotency Tests', () => {
    it('replays cached response for same Idempotency-Key and payload (HTTP-level idempotency)', async () => {
      mockTilledClient.createRefund.mockResolvedValue({
        id: 'rf_tilled_123',
        status: 'succeeded',
        amount: 1000,
        currency: 'usd',
      });

      // First request
      const firstResponse = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'replay-key-1')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_replay_1',
        });

      expect(firstResponse.status).toBe(201);
      const firstRefundId = firstResponse.body.refund.id;

      // Reset mocks
      jest.clearAllMocks();

      // Second request with SAME Idempotency-Key and SAME payload
      const secondResponse = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'replay-key-1')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_replay_1',
        });

      // Should return cached response
      expect(secondResponse.status).toBe(201);
      expect(secondResponse.body.refund.id).toBe(firstRefundId);

      // CRITICAL: Should NOT create new refund row
      const refundCount = await billingPrisma.billing_refunds.count({
        where: { reference_id: 'refund_replay_1' },
      });
      expect(refundCount).toBe(1);

      // CRITICAL: Should NOT call Tilled
      expect(mockTilledClient.createRefund).not.toHaveBeenCalled();
    });

    it('returns existing refund for same reference_id with different Idempotency-Key (domain-level idempotency)', async () => {
      mockTilledClient.createRefund.mockResolvedValue({
        id: 'rf_tilled_123',
        status: 'succeeded',
        amount: 1000,
        currency: 'usd',
      });

      // First request
      const firstResponse = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'domain-key-1')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_domain_dup',
        });

      expect(firstResponse.status).toBe(201);
      const firstRefundId = firstResponse.body.refund.id;

      // Reset mocks
      jest.clearAllMocks();

      // Second request with DIFFERENT Idempotency-Key but SAME reference_id
      const secondResponse = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'domain-key-2') // Different
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_domain_dup', // Same
        });

      // Should return existing refund
      expect(secondResponse.status).toBe(201);
      expect(secondResponse.body.refund.id).toBe(firstRefundId);

      // CRITICAL: Should NOT create new refund row
      const refundCount = await billingPrisma.billing_refunds.count({
        where: { reference_id: 'refund_domain_dup' },
      });
      expect(refundCount).toBe(1);

      // CRITICAL: Should NOT call Tilled
      expect(mockTilledClient.createRefund).not.toHaveBeenCalled();
    });

    it('returns 409 for same Idempotency-Key with different payload', async () => {
      mockTilledClient.createRefund.mockResolvedValue({
        id: 'rf_tilled_123',
        status: 'succeeded',
        amount: 1000,
        currency: 'usd',
      });

      // First request
      await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'conflict-key')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_conflict_1',
        });

      // Second request with SAME Idempotency-Key but DIFFERENT payload
      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'conflict-key')
        .send({
          charge_id: testCharge.id,
          amount_cents: 2000, // Different amount
          reference_id: 'refund_conflict_2', // Different reference_id
        });

      expect(response.status).toBe(409);
      expect(response.body.error).toMatch(/Idempotency-Key.*payload/i);
    });
  });

  describe('Processor Error Handling', () => {
    it('returns 502 on Tilled processor error', async () => {
      mockTilledClient.createRefund.mockRejectedValue(
        Object.assign(new Error('Charge already refunded'), {
          code: 'charge_already_refunded',
          message: 'Charge already refunded',
        })
      );

      const response = await request(app)
        .post('/api/billing/refunds?app_id=trashtech')
        .set('Idempotency-Key', 'test-key-15')
        .send({
          charge_id: testCharge.id,
          amount_cents: 1000,
          reference_id: 'refund_test_15',
        });

      expect(response.status).toBe(502);
      expect(response.body.error).toBe('Payment processor error');
      expect(response.body.message).toContain('Charge already refunded');

      // Verify refund was marked as failed in database
      const refund = await billingPrisma.billing_refunds.findFirst({
        where: { reference_id: 'refund_test_15' },
      });
      expect(refund).not.toBeNull();
      expect(refund.status).toBe('failed');
      expect(refund.failure_code).toBe('charge_already_refunded');
      expect(refund.failure_message).toBe('Charge already refunded');
    });
  });
});
