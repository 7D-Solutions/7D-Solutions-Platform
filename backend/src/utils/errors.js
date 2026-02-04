/**
 * Billing Package - Custom Error Classes
 *
 * Typed errors for HTTP status code mapping and centralized error handling.
 * Security: Production-safe error messages, no sensitive data exposure.
 */

/**
 * Base error class for all billing-related errors
 *
 * @extends Error
 * @property {number} statusCode - HTTP status code
 * @property {boolean} isOperational - True for expected errors (vs programmer errors)
 */
class BillingError extends Error {
  constructor(message, statusCode = 500) {
    super(message);
    this.name = this.constructor.name;
    this.statusCode = statusCode;
    this.isOperational = true; // Expected errors, safe to expose to client
    Error.captureStackTrace(this, this.constructor);
  }
}

/**
 * 404 Not Found - Resource does not exist
 *
 * Use for:
 * - Customer not found
 * - Subscription not found
 * - Payment method not found
 * - Charge not found
 */
class NotFoundError extends BillingError {
  constructor(message = 'Resource not found') {
    super(message, 404);
  }
}

/**
 * 400 Bad Request - Invalid input or validation failure
 *
 * Use for:
 * - Missing required fields
 * - Invalid field formats
 * - Business logic validation failures
 */
class ValidationError extends BillingError {
  constructor(message = 'Validation failed') {
    super(message, 400);
  }
}

/**
 * 409 Conflict - Resource conflict or state violation
 *
 * Use for:
 * - Duplicate records (unique constraint violations)
 * - Invalid state transitions
 * - Idempotency key reuse with different payload
 * - No default payment method
 * - Charge not settled in processor
 */
class ConflictError extends BillingError {
  constructor(message = 'Conflict error') {
    super(message, 409);
  }
}

/**
 * 401 Unauthorized - Authentication or authorization failure
 *
 * Use for:
 * - Invalid webhook signature
 * - Missing authentication credentials
 * - Invalid API key
 */
class UnauthorizedError extends BillingError {
  constructor(message = 'Unauthorized') {
    super(message, 401);
  }
}

/**
 * 502 Bad Gateway - Payment processor error
 *
 * Use for:
 * - Tilled API errors
 * - Payment processing failures
 * - Refund processing failures
 * - External service unavailable
 *
 * @property {string} code - Processor error code (e.g., 'card_declined')
 */
class PaymentProcessorError extends BillingError {
  constructor(message, code = 'processor_error') {
    super(message, 502);
    this.code = code;
  }
}

/**
 * 403 Forbidden - Valid auth, but insufficient permissions
 *
 * Use for:
 * - Multi-tenant isolation violations (accessing another app's resources)
 * - Feature not enabled for app
 */
class ForbiddenError extends BillingError {
  constructor(message = 'Forbidden') {
    super(message, 403);
  }
}

module.exports = {
  BillingError,
  NotFoundError,
  ValidationError,
  ConflictError,
  UnauthorizedError,
  PaymentProcessorError,
  ForbiddenError
};
