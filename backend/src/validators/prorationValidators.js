const { body, param } = require('express-validator');
const { handleValidationErrors } = require('./shared/validationErrorHandler');
const {
  positiveIntParam,
  positiveIntBody,
  isoDateField,
  enumField,
  amountCentsField
} = require('./shared/validationUtils');

const PRORATION_BEHAVIORS = ['create_prorations', 'none', 'always_invoice'];

/**
 * Validator for POST /proration/calculate
 */
const calculateProrationValidator = [
  positiveIntBody('subscription_id'),
  isoDateField('body', 'change_date', { required: true }),
  amountCentsField('body', 'new_price_cents'),
  amountCentsField('body', 'old_price_cents'),
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
  enumField('body', 'proration_behavior', PRORATION_BEHAVIORS),
  handleValidationErrors
];

/**
 * Validator for POST /proration/apply
 */
const applySubscriptionChangeValidator = [
  positiveIntParam('subscription_id'),
  amountCentsField('body', 'new_price_cents', { required: true }),
  amountCentsField('body', 'old_price_cents', { required: true }),
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
  enumField('body', 'proration_behavior', PRORATION_BEHAVIORS),
  isoDateField('body', 'effective_date'),
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
  positiveIntParam('subscription_id'),
  isoDateField('body', 'cancellation_date', { required: true }),
  enumField('body', 'refund_behavior', ['partial_refund', 'account_credit', 'none']),
  handleValidationErrors
];

module.exports = {
  calculateProrationValidator,
  applySubscriptionChangeValidator,
  calculateCancellationRefundValidator
};
