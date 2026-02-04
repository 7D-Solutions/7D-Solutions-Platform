/**
 * Unit tests for error handler middleware
 *
 * Coverage:
 * - BillingError subclasses (NotFoundError, ValidationError, etc.)
 * - Prisma errors (P2002, P2025, P2003, P2014)
 * - Tilled API errors (processor errors with codes)
 * - Production vs development mode
 * - Default 500 error handling
 * - Multi-tenant logging (app_id preservation)
 */

const handleBillingError = require('../../../backend/src/middleware/errorHandler');
const {
  BillingError,
  NotFoundError,
  ValidationError,
  ConflictError,
  UnauthorizedError,
  PaymentProcessorError,
  ForbiddenError
} = require('../../../backend/src/utils/errors');

// Mock logger
jest.mock('@fireproof/infrastructure/utils/logger', () => ({
  warn: jest.fn(),
  error: jest.fn()
}));

const logger = require('@fireproof/infrastructure/utils/logger');

describe('Error Handler Middleware', () => {
  let req, res, next;

  beforeEach(() => {
    // Reset mocks
    jest.clearAllMocks();

    // Mock request object
    req = {
      method: 'POST',
      path: '/api/billing/customers',
      verifiedAppId: 'trashtech',
      params: {},
      query: {}
    };

    // Mock response object
    res = {
      status: jest.fn().mockReturnThis(),
      json: jest.fn()
    };

    // Mock next function
    next = jest.fn();

    // Set test environment
    process.env.NODE_ENV = 'test';
  });

  afterEach(() => {
    delete process.env.NODE_ENV;
  });

  // =====================================================
  // BillingError Subclasses
  // =====================================================

  describe('BillingError subclasses', () => {
    it('should handle NotFoundError with 404 status', () => {
      const error = new NotFoundError('Customer not found');

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(404);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Customer not found'
      });
      expect(logger.warn).toHaveBeenCalled();
    });

    it('should handle ValidationError with 400 status', () => {
      const error = new ValidationError('Invalid email format');

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Invalid email format'
      });
    });

    it('should handle ConflictError with 409 status', () => {
      const error = new ConflictError('Duplicate customer ID');

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(409);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Duplicate customer ID'
      });
    });

    it('should handle UnauthorizedError with 401 status', () => {
      const error = new UnauthorizedError('Invalid webhook signature');

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(401);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Invalid webhook signature'
      });
    });

    it('should handle ForbiddenError with 403 status', () => {
      const error = new ForbiddenError('Cannot access another app resources');

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(403);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Cannot access another app resources'
      });
    });

    it('should handle PaymentProcessorError with 502 status and code', () => {
      const error = new PaymentProcessorError('Card declined', 'card_declined');

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(502);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Card declined',
        code: 'card_declined'
      });
    });

    it('should handle base BillingError with custom status code', () => {
      const error = new BillingError('Custom error', 418);

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(418);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Custom error'
      });
    });
  });

  // =====================================================
  // Prisma Errors
  // =====================================================

  describe('Prisma errors', () => {
    it('should handle P2002 (unique constraint) as 409 Conflict', () => {
      const error = new Error('Unique constraint failed');
      error.code = 'P2002';

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(409);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Duplicate record - resource already exists'
      });
    });

    it('should handle P2025 (record not found) as 404', () => {
      const error = new Error('Record to update not found');
      error.code = 'P2025';

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(404);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Record not found'
      });
    });

    it('should handle P2003 (foreign key constraint) as 400', () => {
      const error = new Error('Foreign key constraint failed');
      error.code = 'P2003';

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Invalid reference - related resource does not exist'
      });
    });

    it('should handle P2014 (relation violation) as 400', () => {
      const error = new Error('Relation violation');
      error.code = 'P2014';

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Cannot delete record with existing dependencies'
      });
    });

    it('should handle unknown Prisma errors as 500 in production', () => {
      process.env.NODE_ENV = 'production';
      const error = new Error('Connection timeout');
      error.code = 'P1001';

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(500);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Database error'
      });
      expect(logger.error).toHaveBeenCalled();
    });

    it('should include details for unknown Prisma errors in development', () => {
      process.env.NODE_ENV = 'development';
      const error = new Error('Connection timeout');
      error.code = 'P1001';

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(500);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Database error',
        details: 'Connection timeout'
      });
    });
  });

  // =====================================================
  // Tilled API Errors
  // =====================================================

  describe('Tilled API errors', () => {
    it('should handle Tilled API errors with code as 502', () => {
      const error = new Error('Insufficient funds');
      error.code = 'insufficient_funds';

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(502);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Payment processor error',
        code: 'insufficient_funds',
        message: 'Insufficient funds'
      });
    });

    it('should handle card_declined error code', () => {
      const error = new Error('Card was declined');
      error.code = 'card_declined';

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(502);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Payment processor error',
        code: 'card_declined',
        message: 'Card was declined'
      });
    });

    it('should handle expired_card error code', () => {
      const error = new Error('Card has expired');
      error.code = 'expired_card';

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(502);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Payment processor error',
        code: 'expired_card',
        message: 'Card has expired'
      });
    });

    it('should NOT treat Prisma errors as Tilled errors', () => {
      const error = new Error('Prisma error');
      error.code = 'P2002';

      handleBillingError(error, req, res, next);

      // Should be handled as Prisma error (409), not Tilled error (502)
      expect(res.status).toHaveBeenCalledWith(409);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Duplicate record - resource already exists'
      });
    });
  });

  // =====================================================
  // Production vs Development Mode
  // =====================================================

  describe('Production vs Development mode', () => {
    it('should return generic message in production mode', () => {
      process.env.NODE_ENV = 'production';
      const error = new Error('Sensitive internal error details');

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(500);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Internal server error'
      });
      expect(res.json).not.toHaveBeenCalledWith(
        expect.objectContaining({ stack: expect.anything() })
      );
    });

    it('should include error message and stack in development mode', () => {
      process.env.NODE_ENV = 'development';
      const error = new Error('Detailed error for debugging');

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(500);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Detailed error for debugging',
        stack: expect.stringContaining('Error: Detailed error for debugging')
      });
    });

    it('should include stack trace in test mode', () => {
      process.env.NODE_ENV = 'test';
      const error = new Error('Test error');

      handleBillingError(error, req, res, next);

      expect(res.json).toHaveBeenCalledWith(
        expect.objectContaining({
          stack: expect.stringContaining('Error: Test error')
        })
      );
    });
  });

  // =====================================================
  // Logging and Multi-Tenant Isolation
  // =====================================================

  describe('Logging and multi-tenant context', () => {
    it('should log operational errors as warnings', () => {
      const error = new NotFoundError('Customer not found');

      handleBillingError(error, req, res, next);

      expect(logger.warn).toHaveBeenCalledWith(
        'Operational error in billing package',
        expect.objectContaining({
          method: 'POST',
          path: '/api/billing/customers',
          app_id: 'trashtech',
          error_name: 'NotFoundError',
          error_message: 'Customer not found'
        })
      );
      expect(logger.error).not.toHaveBeenCalled();
    });

    it('should log non-operational errors as errors with stack trace', () => {
      const error = new Error('Unexpected error');
      error.isOperational = false;

      handleBillingError(error, req, res, next);

      expect(logger.error).toHaveBeenCalledWith(
        'Unexpected error in billing package',
        expect.objectContaining({
          method: 'POST',
          path: '/api/billing/customers',
          app_id: 'trashtech',
          error_name: 'Error',
          error_message: 'Unexpected error',
          stack: expect.stringContaining('Error: Unexpected error')
        })
      );
      expect(logger.warn).not.toHaveBeenCalled();
    });

    it('should preserve app_id from verifiedAppId for multi-tenant tracing', () => {
      req.verifiedAppId = 'apping';
      const error = new ValidationError('Invalid input');

      handleBillingError(error, req, res, next);

      expect(logger.warn).toHaveBeenCalledWith(
        expect.anything(),
        expect.objectContaining({ app_id: 'apping' })
      );
    });

    it('should fallback to params.app_id if verifiedAppId missing', () => {
      req.verifiedAppId = null;
      req.params.app_id = 'trashtech';
      const error = new ValidationError('Invalid input');

      handleBillingError(error, req, res, next);

      expect(logger.warn).toHaveBeenCalledWith(
        expect.anything(),
        expect.objectContaining({ app_id: 'trashtech' })
      );
    });

    it('should fallback to query.app_id if params missing', () => {
      req.verifiedAppId = null;
      req.params = {};
      req.query.app_id = 'apping';
      const error = new ValidationError('Invalid input');

      handleBillingError(error, req, res, next);

      expect(logger.warn).toHaveBeenCalledWith(
        expect.anything(),
        expect.objectContaining({ app_id: 'apping' })
      );
    });

    it('should handle missing app_id gracefully', () => {
      req.verifiedAppId = null;
      req.params = {};
      req.query = {};
      const error = new ValidationError('Invalid input');

      handleBillingError(error, req, res, next);

      expect(logger.warn).toHaveBeenCalledWith(
        expect.anything(),
        expect.objectContaining({ app_id: undefined })
      );
      // Should not crash
      expect(res.status).toHaveBeenCalledWith(400);
    });
  });

  // =====================================================
  // Edge Cases
  // =====================================================

  describe('Edge cases', () => {
    it('should handle errors without messages', () => {
      const error = new NotFoundError();

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(404);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Resource not found' // Default message
      });
    });

    it('should handle null error gracefully', () => {
      const error = null;

      expect(() => {
        handleBillingError(error, req, res, next);
      }).not.toThrow();
    });

    it('should handle error with numeric code (not string)', () => {
      const error = new Error('Database error');
      error.code = 1234; // Numeric code, not string

      handleBillingError(error, req, res, next);

      // Should fall through to default 500 handler
      expect(res.status).toHaveBeenCalledWith(500);
    });

    it('should handle PaymentProcessorError without code', () => {
      const error = new PaymentProcessorError('Unknown processor error');

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(502);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Unknown processor error',
        code: 'processor_error' // Default code
      });
    });

    it('should handle errors with code property that is empty string', () => {
      const error = new Error('Error with empty code');
      error.code = '';

      handleBillingError(error, req, res, next);

      // Should not match Prisma or Tilled patterns, fall to default 500
      expect(res.status).toHaveBeenCalledWith(500);
    });
  });

  // =====================================================
  // Integration Scenarios
  // =====================================================

  describe('Integration scenarios', () => {
    it('should handle multiple error properties correctly', () => {
      const error = new PaymentProcessorError('Payment failed', 'payment_failed');
      error.statusCode = 502;
      error.isOperational = true;

      handleBillingError(error, req, res, next);

      expect(res.status).toHaveBeenCalledWith(502);
      expect(res.json).toHaveBeenCalledWith({
        error: 'Payment failed',
        code: 'payment_failed'
      });
      expect(logger.warn).toHaveBeenCalled(); // isOperational = true
    });

    it('should NOT call next() (terminal middleware)', () => {
      const error = new NotFoundError('Customer not found');

      handleBillingError(error, req, res, next);

      expect(next).not.toHaveBeenCalled();
    });
  });
});
