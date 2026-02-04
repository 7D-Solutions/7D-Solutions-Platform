const { body, query } = require('express-validator');
const { handleValidationErrors } = require('./shared/validationUtils');

/**
 * Validator for POST /usage/record
 */
const recordUsageValidator = [
  body('customer_id')
    .notEmpty()
    .withMessage('customer_id is required')
    .isInt({ min: 1 })
    .withMessage('customer_id must be a positive integer')
    .toInt(),
  body('subscription_id')
    .optional()
    .isInt({ min: 1 })
    .withMessage('subscription_id must be a positive integer if provided')
    .toInt(),
  body('metric_name')
    .notEmpty()
    .withMessage('metric_name is required')
    .trim()
    .escape()
    .isLength({ max: 100 })
    .withMessage('metric_name cannot exceed 100 characters'),
  body('quantity')
    .notEmpty()
    .withMessage('quantity is required')
    .isDecimal()
    .withMessage('quantity must be a decimal number')
    .custom(value => parseFloat(value) >= 0)
    .withMessage('quantity must be non-negative')
    .toFloat(),
  body('unit_price_cents')
    .notEmpty()
    .withMessage('unit_price_cents is required')
    .isInt({ min: 0 })
    .withMessage('unit_price_cents must be a non-negative integer')
    .toInt(),
  body('period_start')
    .notEmpty()
    .withMessage('period_start is required')
    .isISO8601()
    .withMessage('period_start must be a valid ISO 8601 date'),
  body('period_end')
    .notEmpty()
    .withMessage('period_end is required')
    .isISO8601()
    .withMessage('period_end must be a valid ISO 8601 date')
    .custom((value, { req }) => {
      if (new Date(value) <= new Date(req.body.period_start)) {
        throw new Error('period_end must be after period_start');
      }
      return true;
    }),
  body('metadata')
    .optional()
    .isObject()
    .withMessage('metadata must be an object'),
  handleValidationErrors
];

/**
 * Validator for POST /usage/calculate-charges
 */
const calculateUsageChargesValidator = [
  body('customer_id')
    .notEmpty()
    .withMessage('customer_id is required')
    .isInt({ min: 1 })
    .withMessage('customer_id must be a positive integer')
    .toInt(),
  body('subscription_id')
    .optional()
    .isInt({ min: 1 })
    .withMessage('subscription_id must be a positive integer if provided')
    .toInt(),
  body('billing_period_start')
    .notEmpty()
    .withMessage('billing_period_start is required')
    .isISO8601()
    .withMessage('billing_period_start must be a valid ISO 8601 date'),
  body('billing_period_end')
    .notEmpty()
    .withMessage('billing_period_end is required')
    .isISO8601()
    .withMessage('billing_period_end must be a valid ISO 8601 date')
    .custom((value, { req }) => {
      if (new Date(value) <= new Date(req.body.billing_period_start)) {
        throw new Error('billing_period_end must be after billing_period_start');
      }
      return true;
    }),
  body('create_charges')
    .optional()
    .isBoolean()
    .withMessage('create_charges must be a boolean')
    .toBoolean(),
  handleValidationErrors
];

/**
 * Validator for GET /usage/report
 */
const getUsageReportValidator = [
  query('customer_id')
    .notEmpty()
    .withMessage('customer_id is required')
    .isInt({ min: 1 })
    .withMessage('customer_id must be a positive integer')
    .toInt(),
  query('subscription_id')
    .optional()
    .isInt({ min: 1 })
    .withMessage('subscription_id must be a positive integer if provided')
    .toInt(),
  query('start_date')
    .notEmpty()
    .withMessage('start_date is required')
    .isISO8601()
    .withMessage('start_date must be a valid ISO 8601 date'),
  query('end_date')
    .notEmpty()
    .withMessage('end_date is required')
    .isISO8601()
    .withMessage('end_date must be a valid ISO 8601 date')
    .custom((value, { req }) => {
      if (new Date(value) <= new Date(req.query.start_date)) {
        throw new Error('end_date must be after start_date');
      }
      return true;
    }),
  query('include_billed')
    .optional()
    .isBoolean()
    .withMessage('include_billed must be a boolean')
    .toBoolean(),
  query('include_unbilled')
    .optional()
    .isBoolean()
    .withMessage('include_unbilled must be a boolean')
    .toBoolean(),
  handleValidationErrors
];

module.exports = {
  recordUsageValidator,
  calculateUsageChargesValidator,
  getUsageReportValidator
};