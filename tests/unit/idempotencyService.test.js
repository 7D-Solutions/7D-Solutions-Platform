const IdempotencyService = require('../../backend/src/services/IdempotencyService');
const { billingPrisma } = require('../../backend/src/prisma');
const { ConflictError } = require('../../backend/src/utils/errors');

jest.mock('../../backend/src/prisma', () => {
  const mockPrisma = {
    billing_idempotency_keys: {
      findFirst: jest.fn(),
      create: jest.fn(),
      deleteMany: jest.fn(),
    },
  };
  return { billingPrisma: mockPrisma };
});

describe('IdempotencyService', () => {
  let service;

  beforeEach(() => {
    service = new IdempotencyService();
    jest.clearAllMocks();
  });

  describe('computeRequestHash', () => {
    it('should produce consistent hashes for same input', () => {
      const hash1 = service.computeRequestHash('POST', '/charges', { amount: 100 });
      const hash2 = service.computeRequestHash('POST', '/charges', { amount: 100 });
      expect(hash1).toBe(hash2);
    });

    it('should produce different hashes for different input', () => {
      const hash1 = service.computeRequestHash('POST', '/charges', { amount: 100 });
      const hash2 = service.computeRequestHash('POST', '/charges', { amount: 200 });
      expect(hash1).not.toBe(hash2);
    });

    it('should return a 64-character hex string', () => {
      const hash = service.computeRequestHash('POST', '/charges', {});
      expect(hash).toMatch(/^[0-9a-f]{64}$/);
    });
  });

  describe('getIdempotentResponse', () => {
    it('should return null when no record exists', async () => {
      billingPrisma.billing_idempotency_keys.findFirst.mockResolvedValue(null);

      const result = await service.getIdempotentResponse('trashtech', 'key-1', 'hash-abc');

      expect(result).toBeNull();
    });

    it('should return cached response when hash matches', async () => {
      billingPrisma.billing_idempotency_keys.findFirst.mockResolvedValue({
        request_hash: 'hash-abc',
        status_code: 201,
        response_body: { charge: { id: 1 } },
      });

      const result = await service.getIdempotentResponse('trashtech', 'key-1', 'hash-abc');

      expect(result).toEqual({
        statusCode: 201,
        body: { charge: { id: 1 } },
      });
    });

    it('should throw ConflictError when hash does not match', async () => {
      billingPrisma.billing_idempotency_keys.findFirst.mockResolvedValue({
        request_hash: 'hash-different',
        status_code: 201,
        response_body: {},
      });

      await expect(
        service.getIdempotentResponse('trashtech', 'key-1', 'hash-abc')
      ).rejects.toThrow(ConflictError);
    });
  });

  describe('storeIdempotentResponse', () => {
    it('should create a new record with TTL', async () => {
      billingPrisma.billing_idempotency_keys.create.mockResolvedValue({});

      await service.storeIdempotentResponse(
        'trashtech', 'key-1', 'hash-abc', 201, { charge: { id: 1 } }
      );

      expect(billingPrisma.billing_idempotency_keys.create).toHaveBeenCalledWith({
        data: expect.objectContaining({
          app_id: 'trashtech',
          idempotency_key: 'key-1',
          request_hash: 'hash-abc',
          status_code: 201,
          response_body: { charge: { id: 1 } },
          expires_at: expect.any(Date),
        }),
      });

      // Verify TTL is approximately 30 days from now
      const call = billingPrisma.billing_idempotency_keys.create.mock.calls[0][0];
      const ttlMs = call.data.expires_at.getTime() - Date.now();
      const thirtyDaysMs = 30 * 24 * 60 * 60 * 1000;
      expect(ttlMs).toBeGreaterThan(thirtyDaysMs - 5000);
      expect(ttlMs).toBeLessThanOrEqual(thirtyDaysMs);
    });

    it('should handle race condition with matching hash gracefully', async () => {
      const uniqueError = new Error('Unique constraint');
      uniqueError.code = 'P2002';
      billingPrisma.billing_idempotency_keys.create.mockRejectedValue(uniqueError);
      billingPrisma.billing_idempotency_keys.findFirst.mockResolvedValue({
        request_hash: 'hash-abc',
      });

      // Should not throw — same hash means same request, safe to ignore
      await expect(
        service.storeIdempotentResponse('trashtech', 'key-1', 'hash-abc', 201, {})
      ).resolves.toBeUndefined();
    });

    it('should throw ConflictError on race condition with different hash', async () => {
      const uniqueError = new Error('Unique constraint');
      uniqueError.code = 'P2002';
      billingPrisma.billing_idempotency_keys.create.mockRejectedValue(uniqueError);
      billingPrisma.billing_idempotency_keys.findFirst.mockResolvedValue({
        request_hash: 'hash-different',
      });

      await expect(
        service.storeIdempotentResponse('trashtech', 'key-1', 'hash-abc', 201, {})
      ).rejects.toThrow(ConflictError);
    });

    it('should propagate non-unique-constraint errors', async () => {
      const dbError = new Error('Connection lost');
      dbError.code = 'P1001';
      billingPrisma.billing_idempotency_keys.create.mockRejectedValue(dbError);

      await expect(
        service.storeIdempotentResponse('trashtech', 'key-1', 'hash-abc', 201, {})
      ).rejects.toThrow('Connection lost');
    });
  });

  describe('purgeExpiredKeys', () => {
    it('should delete records where expires_at is in the past', async () => {
      billingPrisma.billing_idempotency_keys.deleteMany.mockResolvedValue({ count: 42 });

      const purged = await service.purgeExpiredKeys();

      expect(purged).toBe(42);
      expect(billingPrisma.billing_idempotency_keys.deleteMany).toHaveBeenCalledWith({
        where: {
          expires_at: { lt: expect.any(Date) },
        },
      });

      // Verify the cutoff date is approximately now
      const call = billingPrisma.billing_idempotency_keys.deleteMany.mock.calls[0][0];
      const cutoff = call.where.expires_at.lt.getTime();
      expect(Math.abs(cutoff - Date.now())).toBeLessThan(5000);
    });

    it('should return 0 when no expired records exist', async () => {
      billingPrisma.billing_idempotency_keys.deleteMany.mockResolvedValue({ count: 0 });

      const purged = await service.purgeExpiredKeys();

      expect(purged).toBe(0);
    });
  });
});
