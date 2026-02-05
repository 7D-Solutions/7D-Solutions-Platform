const { body, param, query } = require('express-validator');
const {
  handleValidationErrors,
  positiveIntParam,
  positiveIntBody,
  positiveIntQuery,
  isoDateField,
  enumField,
  amountCentsField
} = require('./shared/validationUtils');

const SUBSCRIPTION_STATUSES = ['active', 'canceled', 'past_due', 'unpaid', 'trialing'];

/**
 * Validator for GET /subscriptions/:id
 */
const getSubscriptionByIdValidator = [
  positiveIntParam('id', 'Subscription ID'),
  handleValidationErrors
];

/**
 * Validator for GET /subscriptions (with optional filters)
 */
const listSubscriptionsValidator = [
  positiveIntQuery('billing_customer_id', 'billing_customer_id'),
  enumField('query', 'status', SUBSCRIPTION_STATUSES),
  handleValidationErrors
];

/**
 * Validator for POST /subscriptions
 */
const createSubscriptionValidator = [
  positiveIntBody('billing_customer_id'),
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
  amountCentsField('body', 'price_cents'),
  enumField('body', 'interval_unit', ['day', 'week', 'month', 'year']),
  body('interval_count')
    .optional()
    .isInt({ min: 1 })
    .withMessage('interval_count must be a positive integer')
    .toInt(),
  isoDateField('body', 'billing_cycle_anchor'),
  isoDateField('body', 'trial_end'),
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
  positiveIntParam('id', 'Subscription ID'),
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
  positiveIntParam('id', 'Subscription ID'),
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
  amountCentsField('body', 'price_cents', { required: false }),
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
