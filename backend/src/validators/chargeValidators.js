const { body, header } = require('express-validator');
const { handleValidationErrors } = require('./shared/validationUtils');

/**
 * Validator for POST /charges/one-time
 */
const createOneTimeChargeValidator = [
  header('idempotency-key')
    .notEmpty()
    .withMessage('Idempotency-Key header is required')
    .trim(),
  body('external_customer_id')
    .notEmpty()
    .withMessage('external_customer_id is required')
    .trim()
    .escape(),
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
    .notEmpty()
    .withMessage('reason is required')
    .trim()
    .escape()
    .isLength({ min: 1, max: 100 })
    .withMessage('reason must be between 1 and 100 characters'),
  body('reference_id')
    .notEmpty()
    .withMessage('reference_id is required')
    .trim()
    .escape()
    .isLength({ min: 1, max: 255 })
    .withMessage('reference_id must be between 1 and 255 characters'),
  body('service_date')
    .optional()
    .isISO8601()
    .withMessage('service_date must be a valid ISO 8601 date'),
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
  createOneTimeChargeValidator
};