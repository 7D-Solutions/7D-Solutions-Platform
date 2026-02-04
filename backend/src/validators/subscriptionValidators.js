const { body, param, query } = require('express-validator');
const { handleValidationErrors } = require('./shared/validationUtils');

/**
 * Validator for GET /subscriptions/:id
 */
const getSubscriptionByIdValidator = [
  param('id')
    .isInt({ min: 1 })
    .withMessage('Subscription ID must be a positive integer'),
  handleValidationErrors
];

/**
 * Validator for GET /subscriptions (with optional filters)
 */
const listSubscriptionsValidator = [
  query('billing_customer_id')
    .optional()
    .isInt({ min: 1 })
    .withMessage('billing_customer_id must be a positive integer')
    .toInt(),
  query('status')
    .optional()
    .isIn(['active', 'canceled', 'past_due', 'unpaid', 'trialing'])
    .withMessage('status must be one of: active, canceled, past_due, unpaid, trialing'),
  handleValidationErrors
];

/**
 * Validator for POST /subscriptions
 */
const createSubscriptionValidator = [
  body('billing_customer_id')
    .notEmpty()
    .withMessage('billing_customer_id is required')
    .isInt({ min: 1 })
    .withMessage('billing_customer_id must be a positive integer')
    .toInt(),
  body('payment_method_id')
    .notEmpty()
    .withMessage('payment_method_id is required')
    .trim(),
  body('plan_id')
    .notEmpty()
    .withMessage('plan_id is required')
    .trim()
    .escape()
    .isLength({ min: 1, max: 100 })
    .withMessage('plan_id must be between 1 and 100 characters'),
  body('plan_name')
    .notEmpty()
    .withMessage('plan_name is required')
    .trim()
    .escape()
    .isLength({ min: 1, max: 255 })
    .withMessage('plan_name must be between 1 and 255 characters'),
  body('price_cents')
    .notEmpty()
    .withMessage('price_cents is required')
    .isInt({ min: 0 })
    .withMessage('price_cents must be a non-negative integer')
    .toInt(),
  body('interval_unit')
    .optional()
    .isIn(['day', 'week', 'month', 'year'])
    .withMessage('interval_unit must be one of: day, week, month, year'),
  body('interval_count')
    .optional()
    .isInt({ min: 1 })
    .withMessage('interval_count must be a positive integer')
    .toInt(),
  body('billing_cycle_anchor')
    .optional()
    .isISO8601()
    .withMessage('billing_cycle_anchor must be a valid ISO 8601 date'),
  body('trial_end')
    .optional()
    .isISO8601()
    .withMessage('trial_end must be a valid ISO 8601 date'),
  body('cancel_at_period_end')
    .optional()
    .isBoolean()
    .withMessage('cancel_at_period_end must be a boolean')
    .toBoolean(),
  body('metadata')
    .optional()
    .isObject()
    .withMessage('metadata must be an object'),
  handleValidationErrors
];

/**
 * Validator for DELETE /subscriptions/:id
 */
const cancelSubscriptionValidator = [
  param('id')
    .isInt({ min: 1 })
    .withMessage('Subscription ID must be a positive integer'),
  query('at_period_end')
    .optional()
    .isIn(['true', 'false'])
    .withMessage('at_period_end must be "true" or "false"'),
  handleValidationErrors
];

/**
 * Validator for PUT /subscriptions/:id
 */
const updateSubscriptionValidator = [
  param('id')
    .isInt({ min: 1 })
    .withMessage('Subscription ID must be a positive integer'),
  body('plan_id')
    .optional()
    .trim()
    .escape()
    .isLength({ min: 1, max: 100 })
    .withMessage('plan_id must be between 1 and 100 characters'),
  body('plan_name')
    .optional()
    .trim()
    .escape()
    .isLength({ min: 1, max: 255 })
    .withMessage('plan_name must be between 1 and 255 characters'),
  body('price_cents')
    .optional()
    .isInt({ min: 0 })
    .withMessage('price_cents must be a non-negative integer')
    .toInt(),
  body('cancel_at_period_end')
    .optional()
    .isBoolean()
    .withMessage('cancel_at_period_end must be a boolean')
    .toBoolean(),
  body('metadata')
    .optional()
    .isObject()
    .withMessage('metadata must be an object'),
  handleValidationErrors
];

module.exports = {
  getSubscriptionByIdValidator,
  listSubscriptionsValidator,
  createSubscriptionValidator,
  cancelSubscriptionValidator,
  updateSubscriptionValidator
};