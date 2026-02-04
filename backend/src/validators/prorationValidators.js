const { body, param } = require('express-validator');
const { handleValidationErrors } = require('./shared/validationUtils');

/**
 * Validator for POST /proration/calculate
 */
const calculateProrationValidator = [
  body('subscription_id')
    .notEmpty()
    .withMessage('subscription_id is required')
    .isInt({ min: 1 })
    .withMessage('subscription_id must be a positive integer')
    .toInt(),
  body('change_date')
    .notEmpty()
    .withMessage('change_date is required')
    .isISO8601()
    .withMessage('change_date must be a valid ISO 8601 date'),
  body('new_price_cents')
    .notEmpty()
    .withMessage('new_price_cents is required')
    .isInt({ min: 0 })
    .withMessage('new_price_cents must be a non-negative integer')
    .toInt(),
  body('old_price_cents')
    .notEmpty()
    .withMessage('old_price_cents is required')
    .isInt({ min: 0 })
    .withMessage('old_price_cents must be a non-negative integer')
    .toInt(),
  body('new_quantity')
    .optional()
    .isInt({ min: 1 })
    .withMessage('new_quantity must be a positive integer')
    .toInt(),
  body('old_quantity')
    .optional()
    .isInt({ min: 1 })
    .withMessage('old_quantity must be a positive integer')
    .toInt(),
  body('proration_behavior')
    .optional()
    .isIn(['create_prorations', 'none', 'always_invoice'])
    .withMessage('proration_behavior must be one of: create_prorations, none, always_invoice'),
  handleValidationErrors
];

/**
 * Validator for POST /proration/apply
 */
const applySubscriptionChangeValidator = [
  param('subscription_id')
    .isInt({ min: 1 })
    .withMessage('subscription_id must be a positive integer'),
  body('new_price_cents')
    .notEmpty()
    .withMessage('new_price_cents is required')
    .isInt({ min: 0 })
    .withMessage('new_price_cents must be a non-negative integer')
    .toInt(),
  body('old_price_cents')
    .notEmpty()
    .withMessage('old_price_cents is required')
    .isInt({ min: 0 })
    .withMessage('old_price_cents must be a non-negative integer')
    .toInt(),
  body('new_quantity')
    .optional()
    .isInt({ min: 1 })
    .withMessage('new_quantity must be a positive integer')
    .toInt(),
  body('old_quantity')
    .optional()
    .isInt({ min: 1 })
    .withMessage('old_quantity must be a positive integer')
    .toInt(),
  body('new_plan_id')
    .optional()
    .trim()
    .escape()
    .isLength({ max: 100 })
    .withMessage('new_plan_id must not exceed 100 characters'),
  body('old_plan_id')
    .optional()
    .trim()
    .escape()
    .isLength({ max: 100 })
    .withMessage('old_plan_id must not exceed 100 characters'),
  body('proration_behavior')
    .optional()
    .isIn(['create_prorations', 'none', 'always_invoice'])
    .withMessage('proration_behavior must be one of: create_prorations, none, always_invoice'),
  body('effective_date')
    .optional()
    .isISO8601()
    .withMessage('effective_date must be a valid ISO 8601 date'),
  body('invoice_immediately')
    .optional()
    .isBoolean()
    .withMessage('invoice_immediately must be a boolean')
    .toBoolean(),
  handleValidationErrors
];

/**
 * Validator for POST /proration/cancellation-refund
 */
const calculateCancellationRefundValidator = [
  param('subscription_id')
    .isInt({ min: 1 })
    .withMessage('subscription_id must be a positive integer'),
  body('cancellation_date')
    .notEmpty()
    .withMessage('cancellation_date is required')
    .isISO8601()
    .withMessage('cancellation_date must be a valid ISO 8601 date'),
  body('refund_behavior')
    .optional()
    .isIn(['partial_refund', 'account_credit', 'none'])
    .withMessage('refund_behavior must be one of: partial_refund, account_credit, none'),
  handleValidationErrors
];

module.exports = {
  calculateProrationValidator,
  applySubscriptionChangeValidator,
  calculateCancellationRefundValidator
};