const { body, header } = require('express-validator');
const { handleValidationErrors } = require('./shared/validationErrorHandler');

/**
 * Validator for POST /refunds
 */
const createRefundValidator = [
  header('idempotency-key')
    .notEmpty()
    .withMessage('Idempotency-Key header is required')
    .trim(),
  body('charge_id')
    .notEmpty()
    .withMessage('charge_id is required')
    .isInt({ min: 1 })
    .withMessage('charge_id must be a positive integer')
    .toInt(),
  body('amount_cents')
    .notEmpty()
    .withMessage('amount_cents is required')
    .isInt({ min: 1 })
    .withMessage('amount_cents must be a positive integer')
    .toInt(),
  body('currency')
    .optional()
    .isIn(['usd', 'cad'])
    .withMessage('currency must be one of: usd, cad'),
  body('reason')
    .optional()
    .trim()
    .escape()
    .isLength({ max: 255 })
    .withMessage('reason must not exceed 255 characters'),
  body('reference_id')
    .notEmpty()
    .withMessage('reference_id is required')
    .trim()
    .escape()
    .isLength({ min: 1, max: 255 })
    .withMessage('reference_id must be between 1 and 255 characters'),
  body('note')
    .optional()
    .trim()
    .escape()
    .isLength({ max: 500 })
    .withMessage('note must not exceed 500 characters'),
  body('metadata')
    .optional()
    .isObject()
    .withMessage('metadata must be an object'),
  handleValidationErrors
];

module.exports = {
  createRefundValidator
};