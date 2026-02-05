const crypto = require('crypto');
const { billingPrisma } = require('../prisma');
const { NotFoundError, ValidationError, ConflictError, UnauthorizedError } = require('../utils/errors');

class IdempotencyService {
  computeRequestHash(method, path, body) {
    const payload = JSON.stringify({ method, path, body });
    return crypto.createHash('sha256').update(payload).digest('hex');
  }

  async getIdempotentResponse(appId, idempotencyKey, requestHash) {
    const record = await billingPrisma.billing_idempotency_keys.findFirst({
      where: {
        app_id: appId,
        idempotency_key: idempotencyKey,
      },
    });

    if (!record) {
      return null;
    }

    // Check if request hash matches
    if (record.request_hash !== requestHash) {
      throw new ConflictError('Idempotency-Key reuse with different payload');
    }

    return {
      statusCode: record.status_code,
      body: record.response_body,
    };
  }

  async storeIdempotentResponse(
    appId,
    idempotencyKey,
    requestHash,
    statusCode,
    responseBody,
    ttlDays = 30
  ) {
    const expiresAt = new Date(Date.now() + ttlDays * 24 * 60 * 60 * 1000);

    try {
      // Try to create new record (prevents race condition overwrites)
      await billingPrisma.billing_idempotency_keys.create({
        data: {
          app_id: appId,
          idempotency_key: idempotencyKey,
          request_hash: requestHash,
          response_body: responseBody,
          status_code: statusCode,
          expires_at: expiresAt,
        },
      });
    } catch (error) {
      // If unique constraint violation, verify hash matches (race condition)
      if (error.code === 'P2002') {
        const existing = await billingPrisma.billing_idempotency_keys.findFirst({
          where: {
            app_id: appId,
            idempotency_key: idempotencyKey,
          },
        });

        if (existing && existing.request_hash !== requestHash) {
          throw new ConflictError('Idempotency-Key reuse with different payload');
        }

        // Hash matches, race condition but same request - safe to ignore
        return;
      }

      // Other error, propagate
      throw error;
    }
  }

  async purgeExpiredKeys() {
    const result = await billingPrisma.billing_idempotency_keys.deleteMany({
      where: {
        expires_at: { lt: new Date() },
      },
    });
    return result.count;
  }
}

module.exports = IdempotencyService;
