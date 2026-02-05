const { body, query } = require('express-validator');
const { handleValidationErrors } = require('./shared/validationErrorHandler');
const {
  positiveIntBody,
  positiveIntQuery,
  isoDateField,
  dateRangeFields,
  amountCentsField
} = require('./shared/validationUtils');

/**
 * Validator for POST /usage/record
 */
const recordUsageValidator = [
  positiveIntBody('customer_id'),
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
  amountCentsField('body', 'unit_price_cents'),
  ...dateRangeFields('body', 'period_start', 'period_end', { required: true }),
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
  positiveIntBody('customer_id'),
  body('subscription_id')
    .optional()
    .isInt({ min: 1 })
    .withMessage('subscription_id must be a positive integer if provided')
    .toInt(),
  ...dateRangeFields('body', 'billing_period_start', 'billing_period_end', { required: true }),
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
  positiveIntQuery('subscription_id'),
  ...dateRangeFields('query', 'start_date', 'end_date', { required: true }),
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
