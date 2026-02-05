const WebhookRetryService = require('../../backend/src/services/WebhookRetryService');
const { calculateNextRetry, BACKOFF_DELAYS_MS, DEFAULT_MAX_ATTEMPTS } = require('../../backend/src/services/WebhookRetryService');
const { billingPrisma } = require('../../backend/src/prisma');

jest.mock('../../backend/src/prisma', () => {
  const mockPrisma = {
    billing_webhooks: {
      findMany: jest.fn(),
      findUnique: jest.fn(),
      update: jest.fn(),
      count: jest.fn()
    },
    billing_webhook_attempts: {
      create: jest.fn()
    }
  };
  return { billingPrisma: mockPrisma };
});

describe('WebhookRetryService', () => {
  let service;
  let mockWebhookService;

  beforeEach(() => {
    mockWebhookService = {
      handleWebhookEvent: jest.fn()
    };
    service = new WebhookRetryService(mockWebhookService);
    jest.clearAllMocks();
  });

  describe('calculateNextRetry', () => {
    it('should return a Date object', () => {
      const result = calculateNextRetry(1);
      expect(result).toBeInstanceOf(Date);
    });

    it('should use correct backoff delays for each attempt', () => {
      for (let attempt = 1; attempt <= BACKOFF_DELAYS_MS.length; attempt++) {
        const before = Date.now();
        const result = calculateNextRetry(attempt);
        const expectedDelay = BACKOFF_DELAYS_MS[attempt - 1];
        const diff = result.getTime() - before;

        // Should be within ±10% of the expected delay (plus small execution time tolerance)
        expect(diff).toBeGreaterThan(expectedDelay * 0.89);
        expect(diff).toBeLessThan(expectedDelay * 1.11 + 100);
      }
    });

    it('should cap delay at the last backoff level for high attempt numbers', () => {
      const before = Date.now();
      const result = calculateNextRetry(10);
      const maxDelay = BACKOFF_DELAYS_MS[BACKOFF_DELAYS_MS.length - 1];
      const diff = result.getTime() - before;

      expect(diff).toBeGreaterThan(maxDelay * 0.89);
      expect(diff).toBeLessThan(maxDelay * 1.11 + 100);
    });

    it('should add jitter (not always the exact same delay)', () => {
      const results = new Set();
      for (let i = 0; i < 20; i++) {
        results.add(calculateNextRetry(1).getTime());
      }
      // With jitter, we should get multiple distinct values
      expect(results.size).toBeGreaterThan(1);
    });
  });

  describe('retryWebhook', () => {
    const makeWebhook = (overrides = {}) => ({
      id: 1,
      app_id: 'trashtech',
      event_id: 'evt_retry_1',
      event_type: 'subscription.updated',
      status: 'failed',
      payload: { id: 'evt_retry_1', type: 'subscription.updated', data: { object: {} } },
      attempt_count: 1,
      next_attempt_at: new Date(Date.now() - 60000),
      dead_at: null,
      ...overrides
    });

    it('should process a retry successfully', async () => {
      const webhook = makeWebhook();
      mockWebhookService.handleWebhookEvent.mockResolvedValue(undefined);
      billingPrisma.billing_webhooks.update.mockResolvedValue({});
      billingPrisma.billing_webhook_attempts.create.mockResolvedValue({});

      const result = await service.retryWebhook(webhook, 5);

      expect(result.status).toBe('processed');
      expect(result.attempt).toBe(2);
      expect(result.eventId).toBe('evt_retry_1');

      // Should have set status to processing first, then processed
      expect(billingPrisma.billing_webhooks.update).toHaveBeenCalledTimes(2);
      expect(billingPrisma.billing_webhooks.update.mock.calls[0][0].data.status).toBe('processing');
      expect(billingPrisma.billing_webhooks.update.mock.calls[1][0].data.status).toBe('processed');

      // Should record success attempt
      expect(billingPrisma.billing_webhook_attempts.create).toHaveBeenCalledWith(
        expect.objectContaining({
          data: expect.objectContaining({
            attempt_number: 2,
            status: 'success'
          })
        })
      );
    });

    it('should schedule next retry on failure when under max attempts', async () => {
      const webhook = makeWebhook({ attempt_count: 2 });
      mockWebhookService.handleWebhookEvent.mockRejectedValue(new Error('DB error'));
      billingPrisma.billing_webhooks.update.mockResolvedValue({});
      billingPrisma.billing_webhook_attempts.create.mockResolvedValue({});

      const result = await service.retryWebhook(webhook, 5);

      expect(result.status).toBe('failed');
      expect(result.attempt).toBe(3);
      expect(result.nextAttempt).toBeInstanceOf(Date);
      expect(result.error).toBe('DB error');

      // Should NOT dead letter
      const failUpdate = billingPrisma.billing_webhooks.update.mock.calls[1][0].data;
      expect(failUpdate.dead_at).toBeNull();
      expect(failUpdate.next_attempt_at).toBeInstanceOf(Date);
    });

    it('should dead letter when max attempts reached', async () => {
      const webhook = makeWebhook({ attempt_count: 4 });
      mockWebhookService.handleWebhookEvent.mockRejectedValue(new Error('Persistent error'));
      billingPrisma.billing_webhooks.update.mockResolvedValue({});
      billingPrisma.billing_webhook_attempts.create.mockResolvedValue({});

      const result = await service.retryWebhook(webhook, 5);

      expect(result.status).toBe('dead');
      expect(result.attempt).toBe(5);

      // Should set dead_at
      const failUpdate = billingPrisma.billing_webhooks.update.mock.calls[1][0].data;
      expect(failUpdate.dead_at).toBeInstanceOf(Date);
      expect(failUpdate.next_attempt_at).toBeNull();
    });

    it('should use error.code when available for error_code', async () => {
      const webhook = makeWebhook();
      const error = new Error('Not found');
      error.code = 'P2025';
      mockWebhookService.handleWebhookEvent.mockRejectedValue(error);
      billingPrisma.billing_webhooks.update.mockResolvedValue({});
      billingPrisma.billing_webhook_attempts.create.mockResolvedValue({});

      await service.retryWebhook(webhook, 5);

      const failUpdate = billingPrisma.billing_webhooks.update.mock.calls[1][0].data;
      expect(failUpdate.error_code).toBe('P2025');
    });
  });

  describe('processRetries', () => {
    it('should query for retryable webhooks and process each', async () => {
      const webhooks = [
        {
          app_id: 'trashtech', event_id: 'evt_1', attempt_count: 1,
          payload: { id: 'evt_1', type: 'subscription.updated', data: { object: {} } },
          status: 'failed', dead_at: null, next_attempt_at: new Date(Date.now() - 60000)
        },
        {
          app_id: 'trashtech', event_id: 'evt_2', attempt_count: 2,
          payload: { id: 'evt_2', type: 'charge.failed', data: { object: {} } },
          status: 'failed', dead_at: null, next_attempt_at: new Date(Date.now() - 60000)
        }
      ];
      billingPrisma.billing_webhooks.findMany.mockResolvedValue(webhooks);
      mockWebhookService.handleWebhookEvent.mockResolvedValue(undefined);
      billingPrisma.billing_webhooks.update.mockResolvedValue({});
      billingPrisma.billing_webhook_attempts.create.mockResolvedValue({});

      const results = await service.processRetries({ appId: 'trashtech' });

      expect(results).toHaveLength(2);
      expect(results[0].status).toBe('processed');
      expect(results[1].status).toBe('processed');
    });

    it('should pass batchSize and appId to query', async () => {
      billingPrisma.billing_webhooks.findMany.mockResolvedValue([]);

      await service.processRetries({ appId: 'trashtech', batchSize: 5 });

      expect(billingPrisma.billing_webhooks.findMany).toHaveBeenCalledWith(
        expect.objectContaining({
          where: expect.objectContaining({
            app_id: 'trashtech',
            status: 'failed',
            dead_at: null
          }),
          take: 5
        })
      );
    });

    it('should return empty array when no retryable webhooks', async () => {
      billingPrisma.billing_webhooks.findMany.mockResolvedValue([]);

      const results = await service.processRetries();
      expect(results).toEqual([]);
    });
  });

  describe('getRetryStats', () => {
    it('should return aggregated stats', async () => {
      billingPrisma.billing_webhooks.count
        .mockResolvedValueOnce(3)   // failed
        .mockResolvedValueOnce(1)   // processing
        .mockResolvedValueOnce(2)   // dead lettered
        .mockResolvedValueOnce(2)   // pending retries
        .mockResolvedValueOnce(50); // total processed

      const stats = await service.getRetryStats('trashtech');

      expect(stats).toEqual({
        failed: 3,
        processing: 1,
        deadLettered: 2,
        pendingRetries: 2,
        totalProcessed: 50
      });
    });
  });

  describe('retryDeadLetter', () => {
    it('should throw if webhook not found', async () => {
      billingPrisma.billing_webhooks.findUnique.mockResolvedValue(null);

      await expect(service.retryDeadLetter('trashtech', 'evt_missing'))
        .rejects.toThrow('Webhook not found: evt_missing');
    });

    it('should throw if webhook is not dead-lettered', async () => {
      billingPrisma.billing_webhooks.findUnique.mockResolvedValue({
        event_id: 'evt_1', app_id: 'trashtech', dead_at: null
      });

      await expect(service.retryDeadLetter('trashtech', 'evt_1'))
        .rejects.toThrow('Webhook evt_1 is not dead-lettered');
    });

    it('should reset dead status and retry', async () => {
      const deadWebhook = {
        app_id: 'trashtech',
        event_id: 'evt_dead',
        attempt_count: 5,
        dead_at: new Date(),
        payload: { id: 'evt_dead', type: 'subscription.updated', data: { object: {} } },
        status: 'failed'
      };

      billingPrisma.billing_webhooks.findUnique
        .mockResolvedValueOnce(deadWebhook)  // initial check
        .mockResolvedValueOnce({ ...deadWebhook, dead_at: null, next_attempt_at: new Date() }); // after reset

      billingPrisma.billing_webhooks.update.mockResolvedValue({});
      billingPrisma.billing_webhook_attempts.create.mockResolvedValue({});
      mockWebhookService.handleWebhookEvent.mockResolvedValue(undefined);

      const result = await service.retryDeadLetter('trashtech', 'evt_dead');

      // Should have reset dead_at first
      expect(billingPrisma.billing_webhooks.update.mock.calls[0][0].data).toMatchObject({
        dead_at: null,
        status: 'failed'
      });

      expect(result.status).toBe('processed');
    });
  });
});
