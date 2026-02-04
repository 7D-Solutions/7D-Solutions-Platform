const { body, param, query } = require('express-validator');
const { handleValidationErrors } = require('./shared/validationUtils');

/**
 * Validator for POST /invoices
 */
const createInvoiceValidator = [
  body('customer_id')
    .notEmpty()
    .withMessage('customer_id is required')
    .isInt({ min: 1 })
    .withMessage('customer_id must be a positive integer')
    .toInt(),
  body('amount_cents')
    .notEmpty()
    .withMessage('amount_cents is required')
    .isInt({ min: 0 })
    .withMessage('amount_cents must be a non-negative integer')
    .toInt(),
  body('subscription_id')
    .optional()
    .isInt({ min: 1 })
    .withMessage('subscription_id must be a positive integer if provided')
    .toInt(),
  body('status')
    .optional()
    .isIn(['draft', 'open', 'paid', 'void', 'uncollectible'])
    .withMessage('status must be one of: draft, open, paid, void, uncollectible'),
  body('currency')
    .optional()
    .isIn(['usd', 'cad', 'eur', 'gbp', 'aud'])
    .withMessage('currency must be one of: usd, cad, eur, gbp, aud'),
  body('due_date')
    .optional()
    .isISO8601()
    .withMessage('due_date must be a valid ISO 8601 date'),
  body('metadata')
    .optional()
    .isObject()
    .withMessage('metadata must be an object'),
  body('billing_period_start')
    .optional()
    .isISO8601()
    .withMessage('billing_period_start must be a valid ISO 8601 date'),
  body('billing_period_end')
    .optional()
    .isISO8601()
    .withMessage('billing_period_end must be a valid ISO 8601 date')
    .custom((value, { req }) => {
      if (req.body.billing_period_start && new Date(value) <= new Date(req.body.billing_period_start)) {
        throw new Error('billing_period_end must be after billing_period_start');
      }
      return true;
    }),
  body('line_item_details')
    .optional()
    .isObject()
    .withMessage('line_item_details must be an object'),
  body('compliance_codes')
    .optional()
    .isObject()
    .withMessage('compliance_codes must be an object'),
  handleValidationErrors
];

/**
 * Validator for GET /invoices/:id
 */
const getInvoiceValidator = [
  param('id')
    .isInt({ min: 1 })
    .withMessage('Invoice ID must be a positive integer'),
  query('include_line_items')
    .optional()
    .isIn(['true', 'false'])
    .withMessage('include_line_items must be "true" or "false"'),
  handleValidationErrors
];

/**
 * Validator for POST /invoices/:id/line-items
 */
const addInvoiceLineItemValidator = [
  param('id')
    .isInt({ min: 1 })
    .withMessage('Invoice ID must be a positive integer'),
  body('line_item_type')
    .notEmpty()
    .withMessage('line_item_type is required')
    .isIn(['subscription', 'usage', 'tax', 'discount', 'fee', 'other'])
    .withMessage('line_item_type must be one of: subscription, usage, tax, discount, fee, other'),
  body('description')
    .notEmpty()
    .withMessage('description is required')
    .trim()
    .escape()
    .isLength({ min: 1, max: 255 })
    .withMessage('description must be between 1 and 255 characters'),
  body('quantity')
    .notEmpty()
    .withMessage('quantity is required')
    .isFloat({ min: 0 })
    .withMessage('quantity must be a non-negative number'),
  body('unit_price_cents')
    .notEmpty()
    .withMessage('unit_price_cents is required')
    .isInt({ min: 0 })
    .withMessage('unit_price_cents must be a non-negative integer')
    .toInt(),
  body('metadata')
    .optional()
    .isObject()
    .withMessage('metadata must be an object'),
  handleValidationErrors
];

/**
 * Validator for POST /invoices/generate-from-subscription
 */
const generateInvoiceFromSubscriptionValidator = [
  body('subscription_id')
    .notEmpty()
    .withMessage('subscription_id is required')
    .isInt({ min: 1 })
    .withMessage('subscription_id must be a positive integer')
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
  body('include_usage')
    .optional()
    .isBoolean()
    .withMessage('include_usage must be a boolean')
    .toBoolean(),
  body('include_tax')
    .optional()
    .isBoolean()
    .withMessage('include_tax must be a boolean')
    .toBoolean(),
  body('include_discounts')
    .optional()
    .isBoolean()
    .withMessage('include_discounts must be a boolean')
    .toBoolean(),
  handleValidationErrors
];

/**
 * Validator for PATCH /invoices/:id/status
 */
const updateInvoiceStatusValidator = [
  param('id')
    .isInt({ min: 1 })
    .withMessage('Invoice ID must be a positive integer'),
  body('status')
    .notEmpty()
    .withMessage('status is required')
    .isIn(['draft', 'open', 'paid', 'void', 'uncollectible'])
    .withMessage('status must be one of: draft, open, paid, void, uncollectible'),
  body('paid_at')
    .optional()
    .isISO8601()
    .withMessage('paid_at must be a valid ISO 8601 date'),
  handleValidationErrors
];

/**
 * Validator for GET /invoices
 */
const listInvoicesValidator = [
  query('customer_id')
    .optional()
    .isInt({ min: 1 })
    .withMessage('customer_id must be a positive integer if provided')
    .toInt(),
  query('subscription_id')
    .optional()
    .isInt({ min: 1 })
    .withMessage('subscription_id must be a positive integer if provided')
    .toInt(),
  query('status')
    .optional()
    .isIn(['draft', 'open', 'paid', 'void', 'uncollectible'])
    .withMessage('status must be one of: draft, open, paid, void, uncollectible'),
  query('start_date')
    .optional()
    .isISO8601()
    .withMessage('start_date must be a valid ISO 8601 date'),
  query('end_date')
    .optional()
    .isISO8601()
    .withMessage('end_date must be a valid ISO 8601 date')
    .custom((value, { req }) => {
      if (req.query.start_date && new Date(value) <= new Date(req.query.start_date)) {
        throw new Error('end_date must be after start_date');
      }
      return true;
    }),
  query('limit')
    .optional()
    .isInt({ min: 1, max: 100 })
    .withMessage('limit must be between 1 and 100')
    .toInt(),
  query('offset')
    .optional()
    .isInt({ min: 0 })
    .withMessage('offset must be a non-negative integer')
    .toInt(),
  handleValidationErrors
];

module.exports = {
  createInvoiceValidator,
  getInvoiceValidator,
  addInvoiceLineItemValidator,
  generateInvoiceFromSubscriptionValidator,
  updateInvoiceStatusValidator,
  listInvoicesValidator
};