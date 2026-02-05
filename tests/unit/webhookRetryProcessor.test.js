const WebhookRetryProcessor = require('../../backend/src/services/helpers/WebhookRetryProcessor');
const { BACKOFF_SCHEDULE_SECONDS, MAX_RETRY_ATTEMPTS } = require('../../backend/src/services/helpers/WebhookRetryProcessor');
const { billingPrisma } = require('../../backend/src/prisma');

jest.mock('../../backend/src/prisma', () => {
  const mockPrisma = {
    billing_webhooks: {
      findMany: jest.fn(),
      update: jest.fn()
    },
    billing_webhook_attempts: {
      create: jest.fn()
    }
  };
  return { billingPrisma: mockPrisma };
});

jest.mock('@fireproof/infrastructure/utils/logger', () => ({
  info: jest.fn(),
  warn: jest.fn(),
  error: jest.fn()
}));

describe('WebhookRetryProcessor', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    billingPrisma.billing_webhooks.update.mockResolvedValue({});
    billingPrisma.billing_webhook_attempts.create.mockResolvedValue({});
  });

  // --- classifyError ---

  describe('classifyError', () => {
    it('should return signature_invalid for signature context', () => {
      expect(WebhookRetryProcessor.classifyError(new Error('bad sig'), 'signature'))
        .toBe('signature_invalid');
    });

    it('should return unknown_event_type for unknown_event context', () => {
      expect(WebhookRetryProcessor.classifyError(new Error('no handler'), 'unknown_event'))
        .toBe('unknown_event_type');
    });

    it('should return validation_error for validation context', () => {
      expect(WebhookRetryProcessor.classifyError(new Error('bad input'), 'validation'))
        .toBe('validation_error');
    });

    it('should return database_error for Prisma P-code errors', () => {
      const err = new Error('record not found');
      err.code = 'P2025';
      expect(WebhookRetryProcessor.classifyError(err)).toBe('database_error');
    });

    it('should return database_error for any P-prefixed code', () => {
      const err = new Error('unique constraint');
      err.code = 'P2002';
      expect(WebhookRetryProcessor.classifyError(err)).toBe('database_error');
    });

    it('should return validation_error for ValidationError instances', () => {
      const err = new Error('invalid');
      err.name = 'ValidationError';
      expect(WebhookRetryProcessor.classifyError(err)).toBe('validation_error');
    });

    it('should return handler_not_found for NotFoundError instances', () => {
      const err = new Error('not found');
      err.name = 'NotFoundError';
      expect(WebhookRetryProcessor.classifyError(err)).toBe('handler_not_found');
    });

    it('should return handler_error for generic errors', () => {
      expect(WebhookRetryProcessor.classifyError(new Error('something broke')))
        .toBe('handler_error');
    });

    it('should prioritize context over error type', () => {
      const err = new Error('bad');
      err.code = 'P2002';
      expect(WebhookRetryProcessor.classifyError(err, 'signature')).toBe('signature_invalid');
    });

    it('should prioritize Prisma code over error name', () => {
      const err = new Error('not found');
      err.name = 'NotFoundError';
      err.code = 'P2025';
      expect(WebhookRetryProcessor.classifyError(err)).toBe('database_error');
    });
  });

  // --- isRetryable ---

  describe('isRetryable', () => {
    it('should return false for signature_invalid', () => {
      expect(WebhookRetryProcessor.isRetryable('signature_invalid')).toBe(false);
    });

    it('should return false for unknown_event_type', () => {
      expect(WebhookRetryProcessor.isRetryable('unknown_event_type')).toBe(false);
    });

    it('should return false for validation_error', () => {
      expect(WebhookRetryProcessor.isRetryable('validation_error')).toBe(false);
    });

    it('should return true for handler_error', () => {
      expect(WebhookRetryProcessor.isRetryable('handler_error')).toBe(true);
    });

    it('should return true for database_error', () => {
      expect(WebhookRetryProcessor.isRetryable('database_error')).toBe(true);
    });

    it('should return true for handler_not_found', () => {
      expect(WebhookRetryProcessor.isRetryable('handler_not_found')).toBe(true);
    });
  });

  // --- calculateNextAttempt ---

  describe('calculateNextAttempt', () => {
    it('should return 30s delay for attempt 1', () => {
      const before = Date.now();
      const result = WebhookRetryProcessor.calculateNextAttempt(1);
      expect(result).toBeInstanceOf(Date);
      const diff = (result.getTime() - before) / 1000;
      expect(diff).toBeGreaterThan(29);
      expect(diff).toBeLessThan(31);
    });

    it('should return 2m delay for attempt 2', () => {
      const before = Date.now();
      const result = WebhookRetryProcessor.calculateNextAttempt(2);
      const diff = (result.getTime() - before) / 1000;
      expect(diff).toBeGreaterThan(119);
      expect(diff).toBeLessThan(121);
    });

    it('should return 15m delay for attempt 3', () => {
      const before = Date.now();
      const result = WebhookRetryProcessor.calculateNextAttempt(3);
      const diff = (result.getTime() - before) / 1000;
      expect(diff).toBeGreaterThan(899);
      expect(diff).toBeLessThan(901);
    });

    it('should return 1h delay for attempt 4', () => {
      const before = Date.now();
      const result = WebhookRetryProcessor.calculateNextAttempt(4);
      const diff = (result.getTime() - before) / 1000;
      expect(diff).toBeGreaterThan(3599);
      expect(diff).toBeLessThan(3601);
    });

    it('should return null when attempt count >= MAX_RETRY_ATTEMPTS', () => {
      expect(WebhookRetryProcessor.calculateNextAttempt(MAX_RETRY_ATTEMPTS)).toBeNull();
    });

    it('should return null for attempts beyond max', () => {
      expect(WebhookRetryProcessor.calculateNextAttempt(MAX_RETRY_ATTEMPTS + 1)).toBeNull();
    });

    it('should cap at last backoff for attempt just under max', () => {
      const result = WebhookRetryProcessor.calculateNextAttempt(4);
      expect(result).toBeInstanceOf(Date);
    });
  });

  // --- scheduleRetry ---

  describe('scheduleRetry', () => {
    it('should record attempt and schedule retry for retryable error', async () => {
      const result = await WebhookRetryProcessor.scheduleRetry({
        appId: 'trashtech',
        eventId: 'evt_1',
        error: new Error('DB timeout'),
        currentAttemptCount: 0
      });

      expect(result.errorCode).toBe('handler_error');
      expect(result.retryable).toBe(true);
      expect(result.dead).toBe(false);
      expect(result.nextAttempt).toBeInstanceOf(Date);

      expect(billingPrisma.billing_webhook_attempts.create).toHaveBeenCalledWith({
        data: expect.objectContaining({
          app_id: 'trashtech',
          event_id: 'evt_1',
          attempt_number: 1,
          status: 'failed',
          error_code: 'handler_error'
        })
      });

      expect(billingPrisma.billing_webhooks.update).toHaveBeenCalledWith({
        where: { event_id_app_id: { event_id: 'evt_1', app_id: 'trashtech' } },
        data: expect.objectContaining({
          status: 'failed',
          error_code: 'handler_error',
          dead_at: null,
          next_attempt_at: expect.any(Date)
        })
      });
    });

    it('should dead-letter non-retryable errors immediately', async () => {
      const result = await WebhookRetryProcessor.scheduleRetry({
        appId: 'trashtech',
        eventId: 'evt_2',
        error: new Error('bad input'),
        errorContext: 'validation',
        currentAttemptCount: 0
      });

      expect(result.errorCode).toBe('validation_error');
      expect(result.retryable).toBe(false);
      expect(result.dead).toBe(true);
      expect(result.nextAttempt).toBeNull();

      expect(billingPrisma.billing_webhooks.update).toHaveBeenCalledWith({
        where: { event_id_app_id: { event_id: 'evt_2', app_id: 'trashtech' } },
        data: expect.objectContaining({
          dead_at: expect.any(Date),
          next_attempt_at: null
        })
      });
    });

    it('should dead-letter when max attempts exceeded for retryable error', async () => {
      const result = await WebhookRetryProcessor.scheduleRetry({
        appId: 'trashtech',
        eventId: 'evt_3',
        error: new Error('persistent failure'),
        currentAttemptCount: MAX_RETRY_ATTEMPTS - 1
      });

      expect(result.retryable).toBe(true);
      expect(result.dead).toBe(true);
      expect(result.nextAttempt).toBeNull();

      expect(billingPrisma.billing_webhooks.update).toHaveBeenCalledWith(
        expect.objectContaining({
          data: expect.objectContaining({
            dead_at: expect.any(Date),
            next_attempt_at: null,
            attempt_count: MAX_RETRY_ATTEMPTS
          })
        })
      );
    });

    it('should classify Prisma errors as database_error', async () => {
      const err = new Error('connection reset');
      err.code = 'P1001';

      const result = await WebhookRetryProcessor.scheduleRetry({
        appId: 'trashtech',
        eventId: 'evt_4',
        error: err,
        currentAttemptCount: 1
      });

      expect(result.errorCode).toBe('database_error');
      expect(result.retryable).toBe(true);
    });

    it('should use errorContext when provided', async () => {
      const result = await WebhookRetryProcessor.scheduleRetry({
        appId: 'trashtech',
        eventId: 'evt_5',
        error: new Error('bad sig'),
        errorContext: 'signature',
        currentAttemptCount: 0
      });

      expect(result.errorCode).toBe('signature_invalid');
      expect(result.retryable).toBe(false);
      expect(result.dead).toBe(true);
    });

    it('should increment attempt_count by 1', async () => {
      await WebhookRetryProcessor.scheduleRetry({
        appId: 'trashtech',
        eventId: 'evt_6',
        error: new Error('oops'),
        currentAttemptCount: 3
      });

      expect(billingPrisma.billing_webhook_attempts.create).toHaveBeenCalledWith({
        data: expect.objectContaining({ attempt_number: 4 })
      });
      expect(billingPrisma.billing_webhooks.update).toHaveBeenCalledWith(
        expect.objectContaining({
          data: expect.objectContaining({ attempt_count: 4 })
        })
      );
    });
  });

  // --- processRetryQueue ---

  describe('processRetryQueue', () => {
    const makeWebhook = (overrides = {}) => ({
      app_id: 'trashtech',
      event_id: `evt_${Math.random().toString(36).slice(2, 8)}`,
      event_type: 'subscription.updated',
      status: 'failed',
      payload: { id: 'evt_1', type: 'subscription.updated', data: { object: {} } },
      attempt_count: 1,
      next_attempt_at: new Date(Date.now() - 60000),
      dead_at: null,
      ...overrides
    });

    it('should query for due webhooks with correct filters', async () => {
      billingPrisma.billing_webhooks.findMany.mockResolvedValue([]);
      const handlerFn = jest.fn();

      await WebhookRetryProcessor.processRetryQueue(handlerFn, { batchSize: 5 });

      expect(billingPrisma.billing_webhooks.findMany).toHaveBeenCalledWith({
        where: {
          status: 'failed',
          next_attempt_at: { lte: expect.any(Date) },
          dead_at: null,
          payload: { not: null }
        },
        orderBy: { next_attempt_at: 'asc' },
        take: 5
      });
    });

    it('should return zero stats when no webhooks are due', async () => {
      billingPrisma.billing_webhooks.findMany.mockResolvedValue([]);
      const handlerFn = jest.fn();

      const stats = await WebhookRetryProcessor.processRetryQueue(handlerFn);

      expect(stats).toEqual({ processed: 0, succeeded: 0, failed: 0, dead: 0 });
    });

    it('should process webhooks and track succeeded', async () => {
      const webhook = makeWebhook({ event_id: 'evt_success' });
      billingPrisma.billing_webhooks.findMany.mockResolvedValue([webhook]);
      const handlerFn = jest.fn().mockResolvedValue(undefined);

      const stats = await WebhookRetryProcessor.processRetryQueue(handlerFn);

      expect(stats.processed).toBe(1);
      expect(stats.succeeded).toBe(1);
      expect(stats.failed).toBe(0);
      expect(handlerFn).toHaveBeenCalledWith('trashtech', webhook.payload);
    });

    it('should mark webhook as processing before calling handler', async () => {
      const webhook = makeWebhook({ event_id: 'evt_proc' });
      billingPrisma.billing_webhooks.findMany.mockResolvedValue([webhook]);
      const handlerFn = jest.fn().mockResolvedValue(undefined);

      await WebhookRetryProcessor.processRetryQueue(handlerFn);

      const firstUpdate = billingPrisma.billing_webhooks.update.mock.calls[0];
      expect(firstUpdate[0].data.status).toBe('processing');
    });

    it('should mark webhook as processed on success', async () => {
      const webhook = makeWebhook({ event_id: 'evt_ok' });
      billingPrisma.billing_webhooks.findMany.mockResolvedValue([webhook]);
      const handlerFn = jest.fn().mockResolvedValue(undefined);

      await WebhookRetryProcessor.processRetryQueue(handlerFn);

      const secondUpdate = billingPrisma.billing_webhooks.update.mock.calls[1];
      expect(secondUpdate[0].data.status).toBe('processed');
      expect(secondUpdate[0].data.next_attempt_at).toBeNull();
    });

    it('should record success attempt on success', async () => {
      const webhook = makeWebhook({ event_id: 'evt_succ_attempt', attempt_count: 2 });
      billingPrisma.billing_webhooks.findMany.mockResolvedValue([webhook]);
      const handlerFn = jest.fn().mockResolvedValue(undefined);

      await WebhookRetryProcessor.processRetryQueue(handlerFn);

      expect(billingPrisma.billing_webhook_attempts.create).toHaveBeenCalledWith({
        data: expect.objectContaining({
          event_id: 'evt_succ_attempt',
          attempt_number: 3,
          status: 'success'
        })
      });
    });

    it('should schedule retry on handler failure', async () => {
      const webhook = makeWebhook({ event_id: 'evt_fail', attempt_count: 1 });
      billingPrisma.billing_webhooks.findMany.mockResolvedValue([webhook]);
      const handlerFn = jest.fn().mockRejectedValue(new Error('handler crashed'));

      const stats = await WebhookRetryProcessor.processRetryQueue(handlerFn);

      expect(stats.processed).toBe(1);
      expect(stats.failed).toBe(1);
      expect(stats.succeeded).toBe(0);
    });

    it('should dead-letter on handler failure when at max attempts', async () => {
      const webhook = makeWebhook({
        event_id: 'evt_dead',
        attempt_count: MAX_RETRY_ATTEMPTS - 1
      });
      billingPrisma.billing_webhooks.findMany.mockResolvedValue([webhook]);
      const handlerFn = jest.fn().mockRejectedValue(new Error('still broken'));

      const stats = await WebhookRetryProcessor.processRetryQueue(handlerFn);

      expect(stats.dead).toBe(1);
      expect(stats.failed).toBe(0);
    });

    it('should process multiple webhooks in batch', async () => {
      const webhooks = [
        makeWebhook({ event_id: 'evt_batch_1' }),
        makeWebhook({ event_id: 'evt_batch_2' }),
        makeWebhook({ event_id: 'evt_batch_3' })
      ];
      billingPrisma.billing_webhooks.findMany.mockResolvedValue(webhooks);
      const handlerFn = jest.fn().mockResolvedValue(undefined);

      const stats = await WebhookRetryProcessor.processRetryQueue(handlerFn);

      expect(stats.processed).toBe(3);
      expect(stats.succeeded).toBe(3);
      expect(handlerFn).toHaveBeenCalledTimes(3);
    });

    it('should handle mixed success and failure in batch', async () => {
      const webhooks = [
        makeWebhook({ event_id: 'evt_mix_ok' }),
        makeWebhook({ event_id: 'evt_mix_fail', attempt_count: 1 })
      ];
      billingPrisma.billing_webhooks.findMany.mockResolvedValue(webhooks);
      const handlerFn = jest.fn()
        .mockResolvedValueOnce(undefined)
        .mockRejectedValueOnce(new Error('nope'));

      const stats = await WebhookRetryProcessor.processRetryQueue(handlerFn);

      expect(stats.processed).toBe(2);
      expect(stats.succeeded).toBe(1);
      expect(stats.failed).toBe(1);
    });

    it('should default batchSize to 10', async () => {
      billingPrisma.billing_webhooks.findMany.mockResolvedValue([]);
      const handlerFn = jest.fn();

      await WebhookRetryProcessor.processRetryQueue(handlerFn);

      expect(billingPrisma.billing_webhooks.findMany).toHaveBeenCalledWith(
        expect.objectContaining({ take: 10 })
      );
    });
  });

  // --- Constants ---

  describe('constants', () => {
    it('should export BACKOFF_SCHEDULE_SECONDS with 5 entries', () => {
      expect(BACKOFF_SCHEDULE_SECONDS).toEqual([30, 120, 900, 3600, 14400]);
    });

    it('should export MAX_RETRY_ATTEMPTS as 5', () => {
      expect(MAX_RETRY_ATTEMPTS).toBe(5);
    });
  });
});
