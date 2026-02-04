/**
 * Idempotency Middleware for Billing Routes
 *
 * Provides idempotency checking and response storage for routes that require
 * idempotent operations (charge creation, refunds, etc.).
 *
 * Usage:
 *   router.post('/charges/one-time',
 *     requireAppId(),
 *     rejectSensitiveData,
 *     createOneTimeChargeValidator,
 *     idempotencyMiddleware('/charges/one-time'),
 *     async (req, res, next) => {
 *       // Route handler - req.idempotency contains { key, hash }
 *       const result = await billingService.createOneTimeCharge(...);
 *
 *       // Store idempotent response
 *       await req.idempotency.store(201, { charge: result });
 *
 *       res.status(201).json({ charge: result });
 *     }
 *   );
 */

const IdempotencyService = require('../services/IdempotencyService');
const { ValidationError, ConflictError } = require('../utils/errors');

const idempotencyService = new IdempotencyService();

/**
 * Create idempotency middleware for a specific route path
 *
 * @param {string} routePath - The route path used for request hash calculation
 * @returns {Function} Express middleware
 */
function createIdempotencyMiddleware(routePath) {
  return async (req, res, next) => {
    try {
      const appId = req.verifiedAppId;
      const idempotencyKey = req.headers['idempotency-key'];

      // Validate idempotency key
      if (!idempotencyKey) {
        throw new ValidationError('Idempotency-Key header is required');
      }

      // Compute request hash
      const requestHash = idempotencyService.computeRequestHash(
        req.method,
        routePath,
        req.body
      );

      // Check for cached response
      const cachedResponse = await idempotencyService.getIdempotentResponse(
        appId,
        idempotencyKey,
        requestHash
      );

      if (cachedResponse) {
        // Return cached response
        return res.status(cachedResponse.statusCode).json(cachedResponse.body);
      }

      // Attach idempotency data to request for later storage
      req.idempotency = {
        key: idempotencyKey,
        hash: requestHash,
        appId,
        store: async (statusCode, responseBody) => {
          await idempotencyService.storeIdempotentResponse(
            appId,
            idempotencyKey,
            requestHash,
            statusCode,
            responseBody
          );
        }
      };

      next();
    } catch (error) {
      next(error);
    }
  };
}

module.exports = createIdempotencyMiddleware;