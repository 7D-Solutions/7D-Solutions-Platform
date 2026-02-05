const { body, param, query } = require('express-validator');
const {
  handleValidationErrors,
  positiveIntParam,
  positiveIntBody,
  positiveIntQuery,
  isoDateField,
  dateRangeFields,
  enumField,
  paginationQuery,
  amountCentsField
} = require('./shared/validationUtils');

const INVOICE_STATUSES = ['draft', 'open', 'paid', 'void', 'uncollectible'];

/**
 * Validator for POST /invoices
 */
const createInvoiceValidator = [
  positiveIntBody('customer_id'),
  amountCentsField('body', 'amount_cents'),
  positiveIntBody('subscription_id').optional(),
  enumField('body', 'status', INVOICE_STATUSES),
  enumField('body', 'currency', ['usd', 'cad', 'eur', 'gbp', 'aud']),
  isoDateField('body', 'due_date'),
  body('metadata')
    .optional()
    .isObject()
    .withMessage('metadata must be an object'),
  ...dateRangeFields('body', 'billing_period_start', 'billing_period_end'),
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
  positiveIntParam('id', 'Invoice ID'),
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
  positiveIntParam('id', 'Invoice ID'),
  enumField('body', 'line_item_type', ['subscription', 'usage', 'tax', 'discount', 'fee', 'other'], { required: true }),
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
  amountCentsField('body', 'unit_price_cents'),
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
  positiveIntBody('subscription_id'),
  ...dateRangeFields('body', 'billing_period_start', 'billing_period_end', { required: true }),
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
  positiveIntParam('id', 'Invoice ID'),
  enumField('body', 'status', INVOICE_STATUSES, { required: true }),
  isoDateField('body', 'paid_at'),
  handleValidationErrors
];

/**
 * Validator for GET /invoices
 */
const listInvoicesValidator = [
  positiveIntQuery('customer_id'),
  positiveIntQuery('subscription_id'),
  enumField('query', 'status', INVOICE_STATUSES),
  ...dateRangeFields('query', 'start_date', 'end_date'),
  ...paginationQuery(),
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
