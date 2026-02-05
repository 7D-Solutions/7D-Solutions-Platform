const { billingPrisma } = require('../../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');

/**
 * WebhookRetryProcessor - Static helpers for webhook retry logic
 *
 * Centralizes error classification, backoff calculation, attempt recording,
 * and retry queue processing. All methods are static.
 */

const BACKOFF_SCHEDULE_SECONDS = [30, 120, 900, 3600, 14400]; // 30s, 2m, 15m, 1h, 4h
const MAX_RETRY_ATTEMPTS = 5;

const NON_RETRYABLE_CODES = new Set([
  'signature_invalid',
  'unknown_event_type',
  'validation_error'
]);

class WebhookRetryProcessor {
  static classifyError(error, context) {
    if (context === 'signature') return 'signature_invalid';
    if (context === 'unknown_event') return 'unknown_event_type';
    if (context === 'validation') return 'validation_error';
    if (error.code && typeof error.code === 'string' && error.code.startsWith('P')) {
      return 'database_error';
    }
    if (error.name === 'ValidationError' || error.constructor?.name === 'ValidationError') {
      return 'validation_error';
    }
    if (error.name === 'NotFoundError' || error.constructor?.name === 'NotFoundError') {
      return 'handler_not_found';
    }
    return 'handler_error';
  }

  static isRetryable(errorCode) {
    return !NON_RETRYABLE_CODES.has(errorCode);
  }

  static calculateNextAttempt(attemptCount) {
    if (attemptCount >= MAX_RETRY_ATTEMPTS) return null;
    const idx = Math.min(attemptCount - 1, BACKOFF_SCHEDULE_SECONDS.length - 1);
    const delaySec = BACKOFF_SCHEDULE_SECONDS[idx];
    return new Date(Date.now() + delaySec * 1000);
  }

  static async scheduleRetry({ appId, eventId, error, errorContext, currentAttemptCount }) {
    const errorCode = WebhookRetryProcessor.classifyError(error, errorContext);
    const retryable = WebhookRetryProcessor.isRetryable(errorCode);
    const attemptNumber = currentAttemptCount + 1;
    let nextAttempt = null;
    let dead = false;
    if (retryable) {
      nextAttempt = WebhookRetryProcessor.calculateNextAttempt(attemptNumber);
      dead = nextAttempt === null;
    } else {
      dead = true;
    }
    await billingPrisma.billing_webhook_attempts.create({
      data: {
        app_id: appId, event_id: eventId, attempt_number: attemptNumber,
        status: 'failed', next_attempt_at: nextAttempt,
        error_code: errorCode, error_message: error.message
      }
    });
    await billingPrisma.billing_webhooks.update({
      where: { event_id_app_id: { event_id: eventId, app_id: appId } },
      data: {
        status: 'failed', error: error.message, error_code: errorCode,
        attempt_count: attemptNumber, last_attempt_at: new Date(),
        next_attempt_at: nextAttempt, dead_at: dead ? new Date() : null,
        processed_at: new Date()
      }
    });
    if (dead && !retryable) {
      logger.warn('Webhook dead-lettered (non-retryable)', { app_id: appId, event_id: eventId, error_code: errorCode });
    } else if (dead) {
      logger.error('Webhook dead-lettered after max attempts', { app_id: appId, event_id: eventId, attempts: attemptNumber });
    } else {
      logger.warn('Webhook retry scheduled', { app_id: appId, event_id: eventId, attempt: attemptNumber, next_attempt_at: nextAttempt, error_code: errorCode });
    }
    return { errorCode, retryable, nextAttempt, dead };
  }

  static async processRetryQueue(handlerFn, options = {}) {
    const { batchSize = 10 } = options;
    const webhooks = await billingPrisma.billing_webhooks.findMany({
      where: { status: 'failed', next_attempt_at: { lte: new Date() }, dead_at: null, payload: { not: null } },
      orderBy: { next_attempt_at: 'asc' }, take: batchSize
    });
    const stats = { processed: 0, succeeded: 0, failed: 0, dead: 0 };
    for (const webhook of webhooks) {
      stats.processed++;
      const { app_id: appId, event_id: eventId, payload, attempt_count: currentAttempts } = webhook;
      await billingPrisma.billing_webhooks.update({
        where: { event_id_app_id: { event_id: eventId, app_id: appId } },
        data: { status: 'processing' }
      });
      try {
        await handlerFn(appId, payload);
        const attemptNumber = currentAttempts + 1;
        await billingPrisma.billing_webhooks.update({
          where: { event_id_app_id: { event_id: eventId, app_id: appId } },
          data: { status: 'processed', attempt_count: attemptNumber, last_attempt_at: new Date(), next_attempt_at: null, processed_at: new Date() }
        });
        await billingPrisma.billing_webhook_attempts.create({
          data: { app_id: appId, event_id: eventId, attempt_number: attemptNumber, status: 'success', next_attempt_at: null }
        });
        logger.info('Webhook retry succeeded', { app_id: appId, event_id: eventId, attempt: attemptNumber });
        stats.succeeded++;
      } catch (error) {
        const result = await WebhookRetryProcessor.scheduleRetry({ appId, eventId, error, currentAttemptCount: currentAttempts });
        if (result.dead) { stats.dead++; } else { stats.failed++; }
      }
    }
    return stats;
  }
}

module.exports = WebhookRetryProcessor;
module.exports.BACKOFF_SCHEDULE_SECONDS = BACKOFF_SCHEDULE_SECONDS;
module.exports.MAX_RETRY_ATTEMPTS = MAX_RETRY_ATTEMPTS;
