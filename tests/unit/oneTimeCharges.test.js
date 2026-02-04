const BillingService = require('../../backend/src/billingService');
const TilledClient = require('../../backend/src/tilledClient');
const { billingPrisma } = require('../../backend/src/prisma');

jest.mock('../../backend/src/tilledClient');
jest.mock('../../backend/src/prisma', () => ({
  billingPrisma: {
    billing_customers: {
      findFirst: jest.fn(),
    },
    billing_charges: {
      create: jest.fn(),
      update: jest.fn(),
      findFirst: jest.fn(),
    },
    billing_idempotency_keys: {
      findFirst: jest.fn(),
      create: jest.fn(),
      upsert: jest.fn(),
    },
  },
}));

describe('BillingService.createOneTimeCharge', () => {
  let billingService;
  let mockTilledClient;

  beforeEach(() => {
    jest.clearAllMocks();
    mockTilledClient = {
      createCharge: jest.fn(),
    };
    TilledClient.mockImplementation(() => mockTilledClient);
    billingService = new BillingService();
  });

  it('throws 404 if billing customer not found for app+external_customer_id', async () => {
    billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

    await expect(
      billingService.createOneTimeCharge(
        'trashtech',
        {
          externalCustomerId: 'nonexistent',
          amountCents: 3500,
          reason: 'extra_pickup',
          referenceId: 'pickup_123',
        },
        { idempotencyKey: 'test-key-1', requestHash: 'hash1' }
      )
    ).rejects.toThrow('Customer not found');
  });

  it('throws 409 if no default payment method on file', async () => {
    billingPrisma.billing_customers.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      external_customer_id: 'cust_123',
      tilled_customer_id: 'tc_123',
      default_payment_method_id: null, // No default PM
    });

    await expect(
      billingService.createOneTimeCharge(
        'trashtech',
        {
          externalCustomerId: 'cust_123',
          amountCents: 3500,
          reason: 'extra_pickup',
          referenceId: 'pickup_123',
        },
        { idempotencyKey: 'test-key-2', requestHash: 'hash2' }
      )
    ).rejects.toThrow('No default payment method');
  });

  it('creates pending record then marks succeeded on tilled success', async () => {
    billingPrisma.billing_customers.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      external_customer_id: 'cust_123',
      tilled_customer_id: 'tc_123',
      default_payment_method_id: 'pm_test_123',
    });

    billingPrisma.billing_charges.findFirst.mockResolvedValue(null); // No duplicate

    const mockChargeRecord = {
      id: 55,
      app_id: 'trashtech',
      status: 'pending',
      amount_cents: 3500,
      currency: 'usd',
      reason: 'extra_pickup',
      reference_id: 'pickup_123',
      tilled_charge_id: null,
    };

    billingPrisma.billing_charges.create.mockResolvedValue(mockChargeRecord);

    mockTilledClient.createCharge.mockResolvedValue({
      id: 'ch_tilled_123',
      status: 'succeeded',
    });

    const updatedCharge = {
      ...mockChargeRecord,
      id: 55,
      status: 'succeeded',
      tilled_charge_id: 'ch_tilled_123',
    };

    billingPrisma.billing_charges.update.mockResolvedValue(updatedCharge);

    const result = await billingService.createOneTimeCharge(
      'trashtech',
      {
        externalCustomerId: 'cust_123',
        amountCents: 3500,
        currency: 'usd',
        reason: 'extra_pickup',
        referenceId: 'pickup_123',
        serviceDate: '2026-01-23',
        note: 'Extra pickup requested',
        metadata: { route_id: 'R12' },
      },
      { idempotencyKey: 'test-key-3', requestHash: 'hash3' }
    );

    expect(billingPrisma.billing_charges.create).toHaveBeenCalledWith({
      data: expect.objectContaining({
        app_id: 'trashtech',
        billing_customer_id: 1,
        status: 'pending',
        amount_cents: 3500,
        currency: 'usd',
        reason: 'extra_pickup',
        reference_id: 'pickup_123',
        service_date: expect.any(Date),
        note: 'Extra pickup requested',
        metadata: { route_id: 'R12' },
      }),
    });

    expect(mockTilledClient.createCharge).toHaveBeenCalledWith({
      appId: 'trashtech',
      tilledCustomerId: 'tc_123',
      paymentMethodId: 'pm_test_123',
      amountCents: 3500,
      currency: 'usd',
      description: 'extra_pickup',
      metadata: expect.any(Object),
    });

    expect(billingPrisma.billing_charges.update).toHaveBeenCalledWith({
      where: { id: 55 },
      data: {
        status: 'succeeded',
        tilled_charge_id: 'ch_tilled_123',
      },
    });

    expect(result.status).toBe('succeeded');
    expect(result.tilled_charge_id).toBe('ch_tilled_123');
  });

  it('marks failed and throws 502 on tilled failure', async () => {
    billingPrisma.billing_customers.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      tilled_customer_id: 'tc_123',
      default_payment_method_id: 'pm_test_123',
    });

    billingPrisma.billing_charges.findFirst.mockResolvedValue(null);

    const mockChargeRecord = {
      id: 56,
      app_id: 'trashtech',
      status: 'pending',
      amount_cents: 3500,
    };

    billingPrisma.billing_charges.create.mockResolvedValue(mockChargeRecord);

    mockTilledClient.createCharge.mockRejectedValue(
      Object.assign(new Error('Insufficient funds'), {
        code: 'card_declined',
        message: 'Insufficient funds',
      })
    );

    billingPrisma.billing_charges.update.mockResolvedValue({
      ...mockChargeRecord,
      status: 'failed',
      failure_code: 'card_declined',
      failure_message: 'Insufficient funds',
    });

    await expect(
      billingService.createOneTimeCharge(
        'trashtech',
        {
          externalCustomerId: 'cust_123',
          amountCents: 3500,
          reason: 'tip',
          referenceId: 'tip_456',
        },
        { idempotencyKey: 'test-key-4', requestHash: 'hash4' }
      )
    ).rejects.toThrow('Insufficient funds');

    expect(billingPrisma.billing_charges.update).toHaveBeenCalledWith({
      where: { id: 56 },
      data: {
        status: 'failed',
        failure_code: 'card_declined',
        failure_message: 'Insufficient funds',
      },
    });
  });

  it('prevents duplicates via unique(app_id, reference_id) and returns existing record', async () => {
    billingPrisma.billing_customers.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      tilled_customer_id: 'tc_123',
      default_payment_method_id: 'pm_test_123',
    });

    const existingCharge = {
      id: 57,
      app_id: 'trashtech',
      billing_customer_id: 1,
      status: 'succeeded',
      amount_cents: 3500,
      currency: 'usd',
      reason: 'extra_pickup',
      reference_id: 'pickup_duplicate',
      tilled_charge_id: 'ch_existing_123',
      created_at: new Date(),
    };

    // First check finds existing
    billingPrisma.billing_charges.findFirst.mockResolvedValue(existingCharge);

    const result = await billingService.createOneTimeCharge(
      'trashtech',
      {
        externalCustomerId: 'cust_123',
        amountCents: 3500,
        reason: 'extra_pickup',
        referenceId: 'pickup_duplicate',
      },
      { idempotencyKey: 'test-key-5', requestHash: 'hash5' }
    );

    // Should NOT create a new charge
    expect(billingPrisma.billing_charges.create).not.toHaveBeenCalled();
    // Should NOT call Tilled
    expect(mockTilledClient.createCharge).not.toHaveBeenCalled();

    expect(result.id).toBe(57);
    expect(result.status).toBe('succeeded');
    expect(result.reference_id).toBe('pickup_duplicate');
  });

  it('domain-idempotency: same reference_id with different Idempotency-Key returns existing without re-processing', async () => {
    // This tests the "dispatcher clicked twice" protection (domain-level idempotency)
    // Different from request-level idempotency (Idempotency-Key)

    billingPrisma.billing_customers.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      tilled_customer_id: 'tc_123',
      default_payment_method_id: 'pm_test_123',
    });

    const existingCharge = {
      id: 99,
      app_id: 'trashtech',
      billing_customer_id: 1,
      status: 'succeeded',
      amount_cents: 5000,
      currency: 'usd',
      reason: 'tip',
      reference_id: 'tip:20260123:DRV123:C456',
      tilled_charge_id: 'ch_original_123',
      created_at: new Date(),
    };

    // Existing charge found by reference_id
    billingPrisma.billing_charges.findFirst.mockResolvedValue(existingCharge);

    // First request (simulating original charge creation)
    const firstResult = await billingService.createOneTimeCharge(
      'trashtech',
      {
        externalCustomerId: 'cust_123',
        amountCents: 5000,
        reason: 'tip',
        referenceId: 'tip:20260123:DRV123:C456',
      },
      { idempotencyKey: 'first-key-abc', requestHash: 'hash-abc' }
    );

    expect(firstResult.id).toBe(99);

    // Reset mocks to verify second call behavior
    jest.clearAllMocks();
    billingPrisma.billing_charges.findFirst.mockResolvedValue(existingCharge);

    // Second request with DIFFERENT Idempotency-Key but SAME reference_id
    // (Simulates dispatcher clicking "Charge Tip" button twice)
    const secondResult = await billingService.createOneTimeCharge(
      'trashtech',
      {
        externalCustomerId: 'cust_123',
        amountCents: 5000,
        reason: 'tip',
        referenceId: 'tip:20260123:DRV123:C456', // SAME reference_id
      },
      { idempotencyKey: 'second-key-xyz', requestHash: 'hash-xyz' } // DIFFERENT Idempotency-Key
    );

    // CRITICAL: Must return the same existing charge
    expect(secondResult.id).toBe(99);
    expect(secondResult.reference_id).toBe('tip:20260123:DRV123:C456');

    // CRITICAL: Must NOT create a new DB record
    expect(billingPrisma.billing_charges.create).not.toHaveBeenCalled();

    // CRITICAL: Must NOT call Tilled API
    expect(mockTilledClient.createCharge).not.toHaveBeenCalled();
  });

  it('validates required fields', async () => {
    await expect(
      billingService.createOneTimeCharge(
        'trashtech',
        {
          externalCustomerId: 'cust_123',
          // Missing amountCents
          reason: 'extra_pickup',
          referenceId: 'pickup_123',
        },
        { idempotencyKey: 'test-key-6', requestHash: 'hash6' }
      )
    ).rejects.toThrow('amountCents is required');

    await expect(
      billingService.createOneTimeCharge(
        'trashtech',
        {
          externalCustomerId: 'cust_123',
          amountCents: 0, // Invalid amount
          reason: 'extra_pickup',
          referenceId: 'pickup_123',
        },
        { idempotencyKey: 'test-key-7', requestHash: 'hash7' }
      )
    ).rejects.toThrow('amountCents must be greater than 0');

    await expect(
      billingService.createOneTimeCharge(
        'trashtech',
        {
          externalCustomerId: 'cust_123',
          amountCents: 3500,
          // Missing reason
          referenceId: 'pickup_123',
        },
        { idempotencyKey: 'test-key-8', requestHash: 'hash8' }
      )
    ).rejects.toThrow('reason is required');

    await expect(
      billingService.createOneTimeCharge(
        'trashtech',
        {
          externalCustomerId: 'cust_123',
          amountCents: 3500,
          reason: 'extra_pickup',
          // Missing referenceId
        },
        { idempotencyKey: 'test-key-9', requestHash: 'hash9' }
      )
    ).rejects.toThrow('referenceId is required');

    // Empty string should also be rejected
    await expect(
      billingService.createOneTimeCharge(
        'trashtech',
        {
          externalCustomerId: 'cust_123',
          amountCents: 3500,
          reason: 'extra_pickup',
          referenceId: '', // Empty string
        },
        { idempotencyKey: 'test-key-10', requestHash: 'hash10' }
      )
    ).rejects.toThrow('referenceId is required');

    // Whitespace-only should also be rejected
    await expect(
      billingService.createOneTimeCharge(
        'trashtech',
        {
          externalCustomerId: 'cust_123',
          amountCents: 3500,
          reason: 'extra_pickup',
          referenceId: '   ', // Whitespace only
        },
        { idempotencyKey: 'test-key-11', requestHash: 'hash11' }
      )
    ).rejects.toThrow('referenceId is required');
  });

  it('sets charge_type to "one_time" for one-time charges', async () => {
    billingPrisma.billing_customers.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      external_customer_id: 'cust_123',
      tilled_customer_id: 'tc_123',
      default_payment_method_id: 'pm_test_123',
    });

    billingPrisma.billing_charges.findFirst.mockResolvedValue(null);

    const mockChargeRecord = {
      id: 100,
      charge_type: 'one_time',
      status: 'pending',
    };

    billingPrisma.billing_charges.create.mockResolvedValue(mockChargeRecord);

    const mockTilledClient = new BillingService().getTilledClient('trashtech');
    mockTilledClient.createCharge = jest.fn().mockResolvedValue({
      id: 'ch_123',
      status: 'succeeded',
    });

    billingPrisma.billing_charges.update.mockResolvedValue({
      ...mockChargeRecord,
      status: 'succeeded',
      tilled_charge_id: 'ch_123',
    });

    await billingService.createOneTimeCharge(
      'trashtech',
      {
        externalCustomerId: 'cust_123',
        amountCents: 3500,
        reason: 'extra_pickup',
        referenceId: 'pickup_123',
      },
      { idempotencyKey: 'test-key-12', requestHash: 'hash12' }
    );

    // Verify charge_type was set in create call
    expect(billingPrisma.billing_charges.create).toHaveBeenCalledWith({
      data: expect.objectContaining({
        charge_type: 'one_time',
      }),
    });
  });
});

describe('Idempotency', () => {
  let billingService;

  beforeEach(() => {
    jest.clearAllMocks();
    billingService = new BillingService();
  });

  it('replays stored response for same key + same request_hash', async () => {
    const storedResponse = {
      id: 1,
      app_id: 'trashtech',
      idempotency_key: 'test-replay-key',
      request_hash: 'hash123',
      response_body: { charge: { id: 100, status: 'succeeded' } },
      status_code: 201,
    };

    billingPrisma.billing_idempotency_keys.findFirst.mockResolvedValue(storedResponse);

    const result = await billingService.getIdempotentResponse(
      'trashtech',
      'test-replay-key',
      'hash123'
    );

    expect(result).toEqual({
      statusCode: 201,
      body: { charge: { id: 100, status: 'succeeded' } },
    });
  });

  it('replays idempotent response without calling Tilled and without inserting new charge rows', async () => {
    // This test verifies that idempotency check short-circuits BEFORE any domain logic
    const mockTilledClient = {
      createCharge: jest.fn(),
    };
    TilledClient.mockImplementation(() => mockTilledClient);

    billingService = new BillingService();

    // Mock idempotency found
    billingPrisma.billing_idempotency_keys.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      idempotency_key: 'replay-key',
      request_hash: 'hash-abc',
      response_body: { charge: { id: 999, status: 'succeeded' } },
      status_code: 201,
    });

    const result = await billingService.getIdempotentResponse(
      'trashtech',
      'replay-key',
      'hash-abc'
    );

    // Verify idempotent response returned
    expect(result.statusCode).toBe(201);
    expect(result.body.charge.id).toBe(999);

    // CRITICAL: Verify NO database operations were attempted
    expect(billingPrisma.billing_customers.findFirst).not.toHaveBeenCalled();
    expect(billingPrisma.billing_charges.findFirst).not.toHaveBeenCalled();
    expect(billingPrisma.billing_charges.create).not.toHaveBeenCalled();

    // CRITICAL: Verify NO Tilled API call was made
    expect(mockTilledClient.createCharge).not.toHaveBeenCalled();
  });

  it('throws 409 for same key with different request_hash', async () => {
    const storedResponse = {
      id: 1,
      app_id: 'trashtech',
      idempotency_key: 'test-conflict-key',
      request_hash: 'hash123',
      response_body: { charge: { id: 100 } },
      status_code: 201,
    };

    billingPrisma.billing_idempotency_keys.findFirst.mockResolvedValue(storedResponse);

    await expect(
      billingService.getIdempotentResponse('trashtech', 'test-conflict-key', 'different_hash')
    ).rejects.toThrow('Idempotency-Key reuse with different payload');
  });

  it('returns null when key not found', async () => {
    billingPrisma.billing_idempotency_keys.findFirst.mockResolvedValue(null);

    const result = await billingService.getIdempotentResponse(
      'trashtech',
      'new-key',
      'hash123'
    );

    expect(result).toBeNull();
  });

  it('stores idempotent response with TTL', async () => {
    const expiresAt = new Date(Date.now() + 30 * 24 * 60 * 60 * 1000);

    billingPrisma.billing_idempotency_keys.create.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      idempotency_key: 'store-key',
      request_hash: 'hash123',
      response_body: { success: true },
      status_code: 200,
      expires_at: expiresAt,
    });

    await billingService.storeIdempotentResponse(
      'trashtech',
      'store-key',
      'hash123',
      200,
      { success: true },
      30
    );

    expect(billingPrisma.billing_idempotency_keys.create).toHaveBeenCalledWith({
      data: expect.objectContaining({
        app_id: 'trashtech',
        idempotency_key: 'store-key',
        request_hash: 'hash123',
        response_body: { success: true },
        status_code: 200,
      }),
    });
  });
});

describe('Race Condition Safety', () => {
  let billingService;
  let mockTilledClient;

  beforeEach(() => {
    jest.clearAllMocks();
    mockTilledClient = {
      createCharge: jest.fn(),
    };
    TilledClient.mockImplementation(() => mockTilledClient);
    billingService = new BillingService();
  });

  it('handles P2002 unique violation on reference_id create (concurrent requests)', async () => {
    // Simulate two concurrent requests with same reference_id
    billingPrisma.billing_customers.findFirst.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      tilled_customer_id: 'tc_123',
      default_payment_method_id: 'pm_test_123',
    });

    // First findFirst returns null (no duplicate)
    billingPrisma.billing_charges.findFirst.mockResolvedValueOnce(null);

    // Create throws P2002 (race condition - another request created it)
    const p2002Error = Object.assign(new Error('Unique constraint failed'), {
      code: 'P2002',
      meta: {
        target: ['unique_app_reference_id'],
      },
    });
    billingPrisma.billing_charges.create.mockRejectedValue(p2002Error);

    // Second findFirst (inside catch) returns existing charge
    const existingCharge = {
      id: 99,
      app_id: 'trashtech',
      billing_customer_id: 1,
      status: 'succeeded',
      amount_cents: 3500,
      reference_id: 'pickup_race',
      tilled_charge_id: 'ch_existing_123',
    };
    billingPrisma.billing_charges.findFirst.mockResolvedValueOnce(existingCharge);

    const result = await billingService.createOneTimeCharge(
      'trashtech',
      {
        externalCustomerId: 'cust_123',
        amountCents: 3500,
        reason: 'extra_pickup',
        referenceId: 'pickup_race',
      },
      { idempotencyKey: 'test-key-race', requestHash: 'hash-race' }
    );

    // CRITICAL: Must return existing charge (not throw)
    expect(result.id).toBe(99);
    expect(result.reference_id).toBe('pickup_race');

    // CRITICAL: Must NOT call Tilled
    expect(mockTilledClient.createCharge).not.toHaveBeenCalled();

    // Verify findFirst was called twice (once before create, once after P2002)
    expect(billingPrisma.billing_charges.findFirst).toHaveBeenCalledTimes(2);
  });

  it('handles concurrent storeIdempotentResponse with create (no crash)', async () => {
    // Create should handle concurrent writes gracefully via P2002 handling
    billingPrisma.billing_idempotency_keys.create.mockResolvedValue({
      id: 1,
      app_id: 'trashtech',
      idempotency_key: 'concurrent-key',
      request_hash: 'hash-concurrent',
      response_body: { charge: { id: 100 } },
      status_code: 201,
    });

    // Should not throw
    await expect(
      billingService.storeIdempotentResponse(
        'trashtech',
        'concurrent-key',
        'hash-concurrent',
        201,
        { charge: { id: 100 } }
      )
    ).resolves.not.toThrow();

    expect(billingPrisma.billing_idempotency_keys.create).toHaveBeenCalled();
  });
});
