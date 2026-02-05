const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');

// Backoff delays for retry attempts (attempt 2 → attempt 5)
const BACKOFF_DELAYS_MS = [
  60_000,       // 1 min
  300_000,      // 5 min
  1_800_000,    // 30 min
  7_200_000     // 2 hours
];

const DEFAULT_MAX_ATTEMPTS = 5;
const DEFAULT_BATCH_SIZE = 10;

/**
 * Calculate the next retry time using exponential backoff with ±10% jitter.
 * @param {number} attemptCount - The current attempt number (1-based)
 * @returns {Date} The next retry timestamp
 */
function calculateNextRetry(attemptCount) {
  const idx = Math.min(attemptCount - 1, BACKOFF_DELAYS_MS.length - 1);
  const delay = BACKOFF_DELAYS_MS[idx];
  const jitter = delay * 0.1 * (Math.random() * 2 - 1); // ±10%
  return new Date(Date.now() + delay + jitter);
}

class WebhookRetryService {
  constructor(webhookService) {
    this.webhookService = webhookService;
  }

  /**
   * Process all retryable webhooks. Host app calls this on a schedule.
   * @param {Object} options
   * @param {string} [options.appId] - Filter to a specific app
   * @param {number} [options.batchSize=10] - Max webhooks to process per run
   * @param {number} [options.maxAttempts=5] - Max attempts before dead letter
   * @returns {Array<Object>} Results per webhook processed
   */
  async processRetries(options = {}) {
    const {
      appId,
      batchSize = DEFAULT_BATCH_SIZE,
      maxAttempts = DEFAULT_MAX_ATTEMPTS
    } = options;

    const webhooks = await billingPrisma.billing_webhooks.findMany({
      where: {
        ...(appId && { app_id: appId }),
        status: 'failed',
        dead_at: null,
        next_attempt_at: { lte: new Date() }
      },
      orderBy: { next_attempt_at: 'asc' },
      take: batchSize
    });

    const results = [];

    for (const webhook of webhooks) {
      const result = await this.retryWebhook(webhook, maxAttempts);
      results.push(result);
    }

    return results;
  }

  /**
   * Retry a single webhook.
   * 1. Set status to 'processing'
   * 2. Call webhookService.handleWebhookEvent(appId, payload)
   * 3. On success: mark 'processed'
   * 4. On failure: increment attempt_count, schedule next retry or dead letter
   * 5. Record attempt in billing_webhook_attempts
   */
  async retryWebhook(webhook, maxAttempts = DEFAULT_MAX_ATTEMPTS) {
    const { app_id: appId, event_id: eventId, attempt_count: currentAttempts, payload } = webhook;
    const attemptNumber = currentAttempts + 1;

    // Mark as processing
    await billingPrisma.billing_webhooks.update({
      where: { event_id_app_id: { event_id: eventId, app_id: appId } },
      data: { status: 'processing' }
    });

    try {
      await this.webhookService.handleWebhookEvent(appId, payload);

      // Success — mark processed
      await billingPrisma.billing_webhooks.update({
        where: { event_id_app_id: { event_id: eventId, app_id: appId } },
        data: {
          status: 'processed',
          attempt_count: attemptNumber,
          last_attempt_at: new Date(),
          next_attempt_at: null,
          processed_at: new Date()
        }
      });

      await billingPrisma.billing_webhook_attempts.create({
        data: {
          app_id: appId,
          event_id: eventId,
          attempt_number: attemptNumber,
          status: 'success',
          next_attempt_at: null
        }
      });

      logger.info('Webhook retry succeeded', { app_id: appId, event_id: eventId, attempt: attemptNumber });

      return { eventId, appId, status: 'processed', attempt: attemptNumber };
    } catch (error) {
      const isDead = attemptNumber >= maxAttempts;
      const nextAttempt = isDead ? null : calculateNextRetry(attemptNumber);

      await billingPrisma.billing_webhooks.update({
        where: { event_id_app_id: { event_id: eventId, app_id: appId } },
        data: {
          status: 'failed',
          error: error.message,
          error_code: error.code || error.constructor.name,
          attempt_count: attemptNumber,
          last_attempt_at: new Date(),
          next_attempt_at: nextAttempt,
          dead_at: isDead ? new Date() : null
        }
      });

      await billingPrisma.billing_webhook_attempts.create({
        data: {
          app_id: appId,
          event_id: eventId,
          attempt_number: attemptNumber,
          status: 'failed',
          next_attempt_at: nextAttempt,
          error_code: error.code || error.constructor.name,
          error_message: error.message
        }
      });

      if (isDead) {
        logger.error('Webhook dead-lettered after max attempts', {
          app_id: appId, event_id: eventId, attempts: attemptNumber
        });
      } else {
        logger.warn('Webhook retry failed, scheduling next attempt', {
          app_id: appId, event_id: eventId, attempt: attemptNumber,
          next_attempt_at: nextAttempt
        });
      }

      return {
        eventId,
        appId,
        status: isDead ? 'dead' : 'failed',
        attempt: attemptNumber,
        error: error.message,
        nextAttempt
      };
    }
  }

  /**
   * Get retry queue statistics.
   * @param {string} appId
   * @returns {Object} Counts by status, pending retries, dead letters
   */
  async getRetryStats(appId) {
    const [failed, processing, deadLettered, pendingRetries, totalProcessed] = await Promise.all([
      billingPrisma.billing_webhooks.count({
        where: { app_id: appId, status: 'failed', dead_at: null }
      }),
      billingPrisma.billing_webhooks.count({
        where: { app_id: appId, status: 'processing' }
      }),
      billingPrisma.billing_webhooks.count({
        where: { app_id: appId, dead_at: { not: null } }
      }),
      billingPrisma.billing_webhooks.count({
        where: {
          app_id: appId,
          status: 'failed',
          dead_at: null,
          next_attempt_at: { lte: new Date() }
        }
      }),
      billingPrisma.billing_webhooks.count({
        where: { app_id: appId, status: 'processed' }
      })
    ]);

    return {
      failed,
      processing,
      deadLettered,
      pendingRetries,
      totalProcessed
    };
  }

  /**
   * Manually retry a specific dead-lettered webhook (admin recovery).
   * Resets dead_at and schedules immediate retry.
   * @param {string} appId
   * @param {string} eventId
   * @returns {Object} Result of the retry attempt
   */
  async retryDeadLetter(appId, eventId) {
    const webhook = await billingPrisma.billing_webhooks.findUnique({
      where: { event_id_app_id: { event_id: eventId, app_id: appId } }
    });

    if (!webhook) {
      throw new Error(`Webhook not found: ${eventId}`);
    }

    if (!webhook.dead_at) {
      throw new Error(`Webhook ${eventId} is not dead-lettered`);
    }

    // Reset dead status and allow one more attempt
    await billingPrisma.billing_webhooks.update({
      where: { event_id_app_id: { event_id: eventId, app_id: appId } },
      data: {
        dead_at: null,
        next_attempt_at: new Date(),
        status: 'failed'
      }
    });

    // Refetch with reset state
    const resetWebhook = await billingPrisma.billing_webhooks.findUnique({
      where: { event_id_app_id: { event_id: eventId, app_id: appId } }
    });

    // Allow one more attempt beyond max (admin override)
    return this.retryWebhook(resetWebhook, resetWebhook.attempt_count + 1);
  }
}

module.exports = WebhookRetryService;
module.exports.calculateNextRetry = calculateNextRetry;
module.exports.BACKOFF_DELAYS_MS = BACKOFF_DELAYS_MS;
module.exports.DEFAULT_MAX_ATTEMPTS = DEFAULT_MAX_ATTEMPTS;
