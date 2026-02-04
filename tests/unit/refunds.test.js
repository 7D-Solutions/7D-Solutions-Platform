jest.mock('../../backend/src/tilledClient');

// Create mock before jest.mock calls
const createMockPrisma = () => ({
  billing_customers: {
    findFirst: jest.fn(),
  },
  billing_charges: {
    findFirst: jest.fn(),
    update: jest.fn(),
  },
  billing_refunds: {
    create: jest.fn(),
    update: jest.fn(),
    findFirst: jest.fn(),
  },
  billing_idempotency_keys: {
    findFirst: jest.fn(),
    upsert: jest.fn(),
    create: jest.fn(),
  },
});

let mockBillingPrisma;

jest.mock('../../backend/src/prisma', () => {
  mockBillingPrisma = createMockPrisma();
  return { billingPrisma: mockBillingPrisma };
});

jest.mock('../../backend/src/prisma.factory', () => ({
  getBillingPrisma: () => mockBillingPrisma || createMockPrisma(),
}));

const BillingService = require('../../backend/src/billingService');
const TilledClient = require('../../backend/src/tilledClient');
const { billingPrisma } = require('../../backend/src/prisma');

describe('RefundService.createRefund', () => {
  let billingService;
  let mockTilledClient;

  beforeEach(() => {
    jest.clearAllMocks();
    mockTilledClient = {
      createRefund: jest.fn(),
    };
    TilledClient.mockImplementation(() => mockTilledClient);
    billingService = new BillingService();
  });

  it('throws error if amountCents is missing', async () => {
    await expect(
      billingService.createRefund(
        'trashtech',
        {
          chargeId: 1,
          referenceId: 'refund_123',
        },
        { idempotencyKey: 'test-key-1', requestHash: 'hash1' }
      )
    ).rejects.toThrow('amountCents is required');
  });

  it('throws error if amountCents is less than or equal to 0', async () => {
    await expect(
      billingService.createRefund(
        'trashtech',
        {
          chargeId: 1,
          amountCents: 0,
          referenceId: 'refund_123',
        },
        { idempotencyKey: 'test-key-2', requestHash: 'hash2' }
      )
    ).rejects.toThrow('amountCents must be greater than 0');

    await expect(
      billingService.createRefund(
        'trashtech',
        {
          chargeId: 1,
          amountCents: -100,
          referenceId: 'refund_123',
        },
        { idempotencyKey: 'test-key-3', requestHash: 'hash3' }
      )
    ).rejects.toThrow('amountCents must be greater than 0');
  });

  it('throws error if referenceId is missing', async () => {
    await expect(
      billingService.createRefund(
        'trashtech',
        {
          chargeId: 1,
          amountCents: 1000,
        },
        { idempotencyKey: 'test-key-4', requestHash: 'hash4' }
      )
    ).rejects.toThrow('referenceId is required');
  });

  it('throws error if referenceId is empty string', async () => {
    await expect(
      billingService.createRefund(
        'trashtech',
        {
          chargeId: 1,
          amountCents: 1000,
          referenceId: '',
        },
        { idempotencyKey: 'test-key-5', requestHash: 'hash5' }
      )
    ).rejects.toThrow('referenceId is required');
  });

  it('throws error if referenceId is whitespace only', async () => {
    await expect(
      billingService.createRefund(
        'trashtech',
        {
          chargeId: 1,
          amountCents: 1000,
          referenceId: '   ',
        },
        { idempotencyKey: 'test-key-6', requestHash: 'hash6' }
      )
    ).rejects.toThrow('referenceId is required');
  });

  it('throws error if chargeId is missing', async () => {
    await expect(
      billingService.createRefund(
        'trashtech',
        {
          amountCents: 1000,
          referenceId: 'refund_123',
        },
        { idempotencyKey: 'test-key-7', requestHash: 'hash7' }
      )
    ).rejects.toThrow('chargeId is required');
  });

  it('throws "Charge not found" if charge does not exist', async () => {
    billingPrisma.billing_charges.findFirst.mockResolvedValue(null);

    await expect(
      billingService.createRefund(
        'trashtech',
        {
          chargeId: 999,
          amountCents: 1000,
          referenceId: 'refund_123',
        },
        { idempotencyKey: 'test-key-8', requestHash: 'hash8' }
      )
    ).rejects.toThrow('Charge not found');
  });

  it('throws "Charge not found" if charge exists but belongs to different app_id (no ID leakage)', async () => {
    billingPrisma.billing_charges.findFirst.mockResolvedValue(null);

    await expect(
      billingService.createRefund(
        'trashtech',
        {
          chargeId: 100,
          amountCents: 1000,
          referenceId: 'refund_123',
        },
        { idempotencyKey: 'test-key-9', requestHash: 'hash9' }
      )
    ).rejects.toThrow('Charge not found');

    // Verify the query included app_id scoping
    expect(billingPrisma.billing_charges.findFirst).toHaveBeenCalledWith({
      where: expect.objectContaining({
        id: 100,
        app_id: 'trashtech',
      }),
      include: expect.any(Object),
    });
  });

  it('throws 409 if charge has no tilled_charge_id (not settled in processor)', async () => {
    billingPrisma.billing_charges.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      billing_customer_id: 10,
      tilled_charge_id: null, // Not settled
      status: 'pending',
      amount_cents: 5000,
    });

    await expect(
      billingService.createRefund(
        'trashtech',
        {
          chargeId: 1,
          amountCents: 1000,
          referenceId: 'refund_123',
        },
        { idempotencyKey: 'test-key-10', requestHash: 'hash10' }
      )
    ).rejects.toThrow('Charge not settled in processor');
  });

  it('returns existing refund for same (app_id, reference_id) WITHOUT calling Tilled', async () => {
    const existingRefund = {
      id: 50,
      app_id: 'trashtech',
      billing_customer_id: 10,
      charge_id: 1,
      tilled_refund_id: 'rf_existing_123',
      status: 'succeeded',
      amount_cents: 1000,
      currency: 'usd',
      reference_id: 'refund_duplicate',
      created_at: new Date(),
    };

    // Domain idempotency check finds existing
    billingPrisma.billing_refunds.findFirst.mockResolvedValue(existingRefund);

    const result = await billingService.createRefund(
      'trashtech',
      {
        chargeId: 1,
        amountCents: 1000,
        referenceId: 'refund_duplicate',
      },
      { idempotencyKey: 'test-key-11', requestHash: 'hash11' }
    );

    // Should return existing refund
    expect(result.id).toBe(50);
    expect(result.reference_id).toBe('refund_duplicate');
    expect(result.status).toBe('succeeded');

    // CRITICAL: Should NOT load charge
    expect(billingPrisma.billing_charges.findFirst).not.toHaveBeenCalled();

    // CRITICAL: Should NOT create new refund
    expect(billingPrisma.billing_refunds.create).not.toHaveBeenCalled();

    // CRITICAL: Should NOT call Tilled
    expect(mockTilledClient.createRefund).not.toHaveBeenCalled();
  });

  it('handles P2002 unique constraint violation on create (race condition) by fetching existing refund and NOT calling Tilled', async () => {
    // Load charge first
    billingPrisma.billing_charges.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      billing_customer_id: 10,
      tilled_charge_id: 'ch_123',
      status: 'succeeded',
      amount_cents: 5000,
    });

    // Domain idempotency check returns null (no existing)
    billingPrisma.billing_refunds.findFirst.mockResolvedValueOnce(null);

    // Create throws P2002 (race condition)
    const p2002Error = Object.assign(new Error('Unique constraint failed'), {
      code: 'P2002',
      meta: {
        target: ['unique_refund_app_reference_id'],
      },
    });
    billingPrisma.billing_refunds.create.mockRejectedValue(p2002Error);

    // Second findFirst (in catch block) returns existing
    const existingRefund = {
      id: 51,
      app_id: 'trashtech',
      billing_customer_id: 10,
      charge_id: 1,
      tilled_refund_id: 'rf_race_123',
      status: 'succeeded',
      amount_cents: 1000,
      reference_id: 'refund_race',
    };
    billingPrisma.billing_refunds.findFirst.mockResolvedValueOnce(existingRefund);

    const result = await billingService.createRefund(
      'trashtech',
      {
        chargeId: 1,
        amountCents: 1000,
        referenceId: 'refund_race',
      },
      { idempotencyKey: 'test-key-12', requestHash: 'hash12' }
    );

    // CRITICAL: Must return existing refund
    expect(result.id).toBe(51);
    expect(result.reference_id).toBe('refund_race');

    // CRITICAL: Must NOT call Tilled
    expect(mockTilledClient.createRefund).not.toHaveBeenCalled();

    // Verify findFirst was called twice (domain check + race recovery)
    expect(billingPrisma.billing_refunds.findFirst).toHaveBeenCalledTimes(2);
  });

  it('creates pending refund, calls Tilled, and updates to succeeded on success', async () => {
    billingPrisma.billing_charges.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      billing_customer_id: 10,
      tilled_charge_id: 'ch_123',
      status: 'succeeded',
      amount_cents: 5000,
    });

    // No existing refund
    billingPrisma.billing_refunds.findFirst.mockResolvedValue(null);

    const mockRefundRecord = {
      id: 60,
      app_id: 'trashtech',
      billing_customer_id: 10,
      charge_id: 1,
      tilled_charge_id: 'ch_123',
      status: 'pending',
      amount_cents: 1000,
      currency: 'usd',
      reference_id: 'refund_success',
      reason: 'requested_by_customer',
      note: 'Customer requested refund',
      metadata: { ticket_id: 'T123' },
    };

    billingPrisma.billing_refunds.create.mockResolvedValue(mockRefundRecord);

    mockTilledClient.createRefund.mockResolvedValue({
      id: 'rf_tilled_123',
      status: 'succeeded',
      amount: 1000,
      currency: 'usd',
    });

    const updatedRefund = {
      ...mockRefundRecord,
      status: 'succeeded',
      tilled_refund_id: 'rf_tilled_123',
    };

    billingPrisma.billing_refunds.update.mockResolvedValue(updatedRefund);

    const result = await billingService.createRefund(
      'trashtech',
      {
        chargeId: 1,
        amountCents: 1000,
        currency: 'usd',
        reason: 'requested_by_customer',
        referenceId: 'refund_success',
        note: 'Customer requested refund',
        metadata: { ticket_id: 'T123' },
      },
      { idempotencyKey: 'test-key-13', requestHash: 'hash13' }
    );

    // Verify charge was loaded with app_id scoping
    expect(billingPrisma.billing_charges.findFirst).toHaveBeenCalledWith({
      where: {
        id: 1,
        app_id: 'trashtech',
      },
      include: { customer: true },
    });

    // Verify pending refund was created
    expect(billingPrisma.billing_refunds.create).toHaveBeenCalledWith({
      data: expect.objectContaining({
        app_id: 'trashtech',
        billing_customer_id: 10,
        charge_id: 1,
        tilled_charge_id: 'ch_123',
        status: 'pending',
        amount_cents: 1000,
        currency: 'usd',
        reason: 'requested_by_customer',
        reference_id: 'refund_success',
        note: 'Customer requested refund',
        metadata: { ticket_id: 'T123' },
      }),
    });

    // Verify Tilled was called
    expect(mockTilledClient.createRefund).toHaveBeenCalledWith({
      appId: 'trashtech',
      tilledChargeId: 'ch_123',
      amountCents: 1000,
      currency: 'usd',
      reason: 'requested_by_customer',
      metadata: expect.any(Object),
    });

    // Verify refund was updated to succeeded
    expect(billingPrisma.billing_refunds.update).toHaveBeenCalledWith({
      where: { id: 60 },
      data: {
        status: 'succeeded',
        tilled_refund_id: 'rf_tilled_123',
      },
    });

    // Verify result
    expect(result.status).toBe('succeeded');
    expect(result.tilled_refund_id).toBe('rf_tilled_123');
  });

  it('updates refund to failed with failure_code/message and rethrows on Tilled failure', async () => {
    billingPrisma.billing_charges.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      billing_customer_id: 10,
      tilled_charge_id: 'ch_123',
      status: 'succeeded',
      amount_cents: 5000,
    });

    billingPrisma.billing_refunds.findFirst.mockResolvedValue(null);

    const mockRefundRecord = {
      id: 61,
      app_id: 'trashtech',
      charge_id: 1,
      status: 'pending',
      amount_cents: 1000,
      reference_id: 'refund_fail',
    };

    billingPrisma.billing_refunds.create.mockResolvedValue(mockRefundRecord);

    // Tilled API failure
    mockTilledClient.createRefund.mockRejectedValue(
      Object.assign(new Error('Charge already refunded'), {
        code: 'charge_already_refunded',
        message: 'Charge already refunded',
      })
    );

    billingPrisma.billing_refunds.update.mockResolvedValue({
      ...mockRefundRecord,
      status: 'failed',
      failure_code: 'charge_already_refunded',
      failure_message: 'Charge already refunded',
    });

    await expect(
      billingService.createRefund(
        'trashtech',
        {
          chargeId: 1,
          amountCents: 1000,
          referenceId: 'refund_fail',
        },
        { idempotencyKey: 'test-key-14', requestHash: 'hash14' }
      )
    ).rejects.toThrow('Charge already refunded');

    // Verify refund was marked as failed
    expect(billingPrisma.billing_refunds.update).toHaveBeenCalledWith({
      where: { id: 61 },
      data: {
        status: 'failed',
        failure_code: 'charge_already_refunded',
        failure_message: 'Charge already refunded',
      },
    });
  });

  it('defaults currency to "usd" if not provided', async () => {
    billingPrisma.billing_charges.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      billing_customer_id: 10,
      tilled_charge_id: 'ch_123',
      status: 'succeeded',
      amount_cents: 5000,
    });

    billingPrisma.billing_refunds.findFirst.mockResolvedValue(null);

    const mockRefundRecord = {
      id: 62,
      status: 'pending',
      amount_cents: 1000,
    };

    billingPrisma.billing_refunds.create.mockResolvedValue(mockRefundRecord);

    mockTilledClient.createRefund.mockResolvedValue({
      id: 'rf_123',
      status: 'succeeded',
    });

    billingPrisma.billing_refunds.update.mockResolvedValue({
      ...mockRefundRecord,
      status: 'succeeded',
      tilled_refund_id: 'rf_123',
    });

    await billingService.createRefund(
      'trashtech',
      {
        chargeId: 1,
        amountCents: 1000,
        referenceId: 'refund_default_currency',
        // currency not provided
      },
      { idempotencyKey: 'test-key-15', requestHash: 'hash15' }
    );

    // Verify create was called with currency = 'usd'
    expect(billingPrisma.billing_refunds.create).toHaveBeenCalledWith({
      data: expect.objectContaining({
        currency: 'usd',
      }),
    });

    // Verify Tilled was called with currency = 'usd'
    expect(mockTilledClient.createRefund).toHaveBeenCalledWith(
      expect.objectContaining({
        currency: 'usd',
      })
    );
  });
});

describe('Idempotency for Refunds', () => {
  let billingService;

  beforeEach(() => {
    jest.clearAllMocks();
    billingService = new BillingService();
  });

  it('storeIdempotentResponse creates idempotency key for refund endpoints', async () => {
    const expiresAt = new Date(Date.now() + 30 * 24 * 60 * 60 * 1000);

    billingPrisma.billing_idempotency_keys.create.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      idempotency_key: 'refund-key',
      request_hash: 'hash123',
      response_body: { refund: { id: 100 } },
      status_code: 201,
      expires_at: expiresAt,
    });

    await billingService.storeIdempotentResponse(
      'trashtech',
      'refund-key',
      'hash123',
      201,
      { refund: { id: 100 } },
      30
    );

    expect(billingPrisma.billing_idempotency_keys.create).toHaveBeenCalledWith({
      data: expect.objectContaining({
        app_id: 'trashtech',
        idempotency_key: 'refund-key',
        request_hash: 'hash123',
        response_body: { refund: { id: 100 } },
        status_code: 201,
      }),
    });
  });
});
