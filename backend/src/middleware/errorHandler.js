/**
 * Billing Package - Centralized Error Handler Middleware
 *
 * Maps all errors to appropriate HTTP responses with production-safe messages.
 * Security: No stack traces in production, preserves multi-tenant isolation.
 *
 * Mount this middleware LAST in Express app (after all routes):
 * app.use('/api/billing', billingRoutes);
 * app.use(handleBillingError);
 */

const logger = require('@fireproof/infrastructure/utils/logger');
const {
  BillingError,
  NotFoundError,
  ValidationError,
  ConflictError,
  UnauthorizedError,
  PaymentProcessorError,
  ForbiddenError
} = require('../utils/errors');

/**
 * Centralized error handler for billing package
 *
 * Error precedence (checked in order):
 * 1. BillingError subclasses (typed errors from services)
 * 2. Prisma errors (database constraint violations)
 * 3. Tilled API errors (payment processor failures)
 * 4. Default 500 Internal Server Error
 *
 * @param {Error} err - Error object
 * @param {Object} req - Express request object
 * @param {Object} res - Express response object
 * @param {Function} next - Express next middleware function
 */
function handleBillingError(err, req, res, next) {
  // Handle null/undefined errors gracefully
  if (!err) {
    logger.error('Error handler called with null/undefined error');
    return res.status(500).json({ error: 'Internal server error' });
  }

  // Log error with request context (preserve app_id for multi-tenant tracing)
  const logContext = {
    method: req.method,
    path: req.path,
    app_id: req.verifiedAppId || req.params?.app_id || req.query?.app_id,
    error_name: err.name,
    error_message: err.message
  };

  // Log operational errors as warnings, programmer errors as errors
  if (err.isOperational) {
    logger.warn('Operational error in billing package', logContext);
  } else {
    logger.error('Unexpected error in billing package', { ...logContext, stack: err.stack });
  }

  // 1. Handle BillingError subclasses (typed errors from services)
  if (err instanceof BillingError) {
    return res.status(err.statusCode).json({
      error: err.message,
      ...(err.code && { code: err.code }) // Include error code for PaymentProcessorError
    });
  }

  // 2. Handle Prisma errors (database constraint violations)
  if (err.code && typeof err.code === 'string' && err.code.startsWith('P')) {
    switch (err.code) {
      case 'P2002': // Unique constraint violation
        return res.status(409).json({
          error: 'Duplicate record - resource already exists'
        });

      case 'P2025': // Record not found
        return res.status(404).json({
          error: 'Record not found'
        });

      case 'P2003': // Foreign key constraint violation
        return res.status(400).json({
          error: 'Invalid reference - related resource does not exist'
        });

      case 'P2014': // Relation violation
        return res.status(400).json({
          error: 'Cannot delete record with existing dependencies'
        });

      default:
        // Other Prisma errors (connection, timeout, etc.)
        logger.error('Prisma database error', { code: err.code, message: err.message });
        return res.status(500).json({
          error: 'Database error',
          ...(process.env.NODE_ENV !== 'production' && { details: err.message })
        });
    }
  }

  // 3. Handle Tilled API errors (payment processor failures)
  // Tilled errors have error.code but NOT Prisma codes (don't start with 'P')
  if (err.code && typeof err.code === 'string' && !err.code.startsWith('P')) {
    return res.status(502).json({
      error: 'Payment processor error',
      code: err.code,
      message: err.message
    });
  }

  // 4. Default: 500 Internal Server Error
  // Production: Generic message (security - no information disclosure)
  // Development: Include error message and stack trace for debugging
  const isProduction = process.env.NODE_ENV === 'production';

  return res.status(500).json({
    error: isProduction ? 'Internal server error' : err.message,
    ...(isProduction === false && { stack: err.stack })
  });
}

module.exports = handleBillingError;
