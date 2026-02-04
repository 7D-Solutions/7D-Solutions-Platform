/**
 * Request Validators for Billing Routes
 *
 * Uses express-validator for centralized validation with input sanitization.
 *
 * Security features:
 * - Email validation using .isEmail() (RFC 5322 compliant)
 * - XSS prevention via .trim().escape() on text fields
 * - Negative amount prevention via .isInt({ min: 1 })
 * - Consistent error messages via .withMessage()
 *
 * Reference: SECURITY-REVIEW-SEPARATION-OF-CONCERNS.md lines 360-370
 */

const { body, param, query, header, validationResult } = require('express-validator');

/**
 * Middleware to handle validation errors
 * Returns 400 with array of error messages if validation fails
 */
const handleValidationErrors = (req, res, next) => {
  const errors = validationResult(req);
  if (!errors.isEmpty()) {
    return res.status(400).json({
      error: 'Validation failed',
      details: errors.array().map(err => ({
        field: err.path || err.param,
        message: err.msg
      }))
    });
  }
  next();
};

// ============================================================================
// CUSTOMER VALIDATORS
// ============================================================================

/**
 * Validator for GET /customers/:id
 */
const getCustomerByIdValidator = [
  param('id')
    .isInt({ min: 1 })
    .withMessage('Customer ID must be a positive integer'),
  handleValidationErrors
];

/**
 * Validator for GET /customers (by external_customer_id)
 */
const getCustomerByExternalIdValidator = [
  query('external_customer_id')
    .notEmpty()
    .withMessage('external_customer_id is required')
    .trim()
    .escape(),
  handleValidationErrors
];

/**
 * Validator for POST /customers
 */
const createCustomerValidator = [
  body('email')
    .notEmpty()
    .withMessage('Email is required')
    .isEmail()
    .withMessage('Invalid email format')
    .normalizeEmail(),
  body('name')
    .notEmpty()
    .withMessage('Name is required')
    .trim()
    .escape()
    .isLength({ min: 1, max: 255 })
    .withMessage('Name must be between 1 and 255 characters'),
  body('external_customer_id')
    .optional()
    .trim()
    .escape()
    .isLength({ max: 255 })
    .withMessage('external_customer_id must not exceed 255 characters'),
  body('metadata')
    .optional()
    .isObject()
    .withMessage('metadata must be an object'),
  handleValidationErrors
];

/**
 * Validator for POST /customers/:id/default-payment-method
 */
const setDefaultPaymentMethodValidator = [
  param('id')
    .isInt({ min: 1 })
    .withMessage('Customer ID must be a positive integer'),
  body('payment_method_id')
    .notEmpty()
    .withMessage('payment_method_id is required')
    .trim(),
  body('payment_method_type')
    .notEmpty()
    .withMessage('payment_method_type is required')
    .isIn(['card', 'ach_debit', 'eft_debit'])
    .withMessage('payment_method_type must be one of: card, ach_debit, eft_debit'),
  handleValidationErrors
];

/**
 * Validator for PUT /customers/:id
 */
const updateCustomerValidator = [
  param('id')
    .isInt({ min: 1 })
    .withMessage('Customer ID must be a positive integer'),
  body('email')
    .optional()
    .isEmail()
    .withMessage('Invalid email format')
    .normalizeEmail(),
  body('name')
    .optional()
    .trim()
    .escape()
    .isLength({ min: 1, max: 255 })
    .withMessage('Name must be between 1 and 255 characters'),
  body('metadata')
    .optional()
    .isObject()
    .withMessage('metadata must be an object'),
  handleValidationErrors
];

// ============================================================================
// BILLING STATE VALIDATOR
// ============================================================================

/**
 * Validator for GET /state
 */
const getBillingStateValidator = [
  query('external_customer_id')
    .notEmpty()
    .withMessage('external_customer_id is required')
    .trim()
    .escape(),
  handleValidationErrors
];

// ============================================================================
// PAYMENT METHOD VALIDATORS
// ============================================================================

/**
 * Validator for GET /payment-methods
 */
const listPaymentMethodsValidator = [
  query('billing_customer_id')
    .notEmpty()
    .withMessage('billing_customer_id is required')
    .isInt({ min: 1 })
    .withMessage('billing_customer_id must be a positive integer')
    .toInt(),
  handleValidationErrors
];

/**
 * Validator for POST /payment-methods
 */
const addPaymentMethodValidator = [
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
  handleValidationErrors
];

/**
 * Validator for PUT /payment-methods/:id/default
 */
const setDefaultPaymentMethodByIdValidator = [
  param('id')
    .notEmpty()
    .withMessage('Payment method ID is required')
    .trim(),
  body('billing_customer_id')
    .notEmpty()
    .withMessage('billing_customer_id is required')
    .isInt({ min: 1 })
    .withMessage('billing_customer_id must be a positive integer')
    .toInt(),
  handleValidationErrors
];

/**
 * Validator for DELETE /payment-methods/:id
 */
const deletePaymentMethodValidator = [
  param('id')
    .notEmpty()
    .withMessage('Payment method ID is required')
    .trim(),
  query('billing_customer_id')
    .notEmpty()
    .withMessage('billing_customer_id is required')
    .isInt({ min: 1 })
    .withMessage('billing_customer_id must be a positive integer')
    .toInt(),
  handleValidationErrors
];

// ============================================================================
// SUBSCRIPTION VALIDATORS
// ============================================================================

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

// ============================================================================
// ONE-TIME CHARGE VALIDATORS
// ============================================================================

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

// ============================================================================
// REFUND VALIDATORS
// ============================================================================

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

// ============================================================================
// TAX VALIDATORS (PHASE 1)
// ============================================================================

/**
 * Validator for GET /tax-rates/:jurisdictionCode
 */
const getTaxRatesByJurisdictionValidator = [
  param('jurisdictionCode')
    .notEmpty()
    .withMessage('Jurisdiction code is required')
    .trim()
    .isLength({ min: 2, max: 20 })
    .withMessage('Jurisdiction code must be between 2 and 20 characters')
    .matches(/^[A-Z0-9-]+$/)
    .withMessage('Jurisdiction code must contain only uppercase letters, numbers, and hyphens'),
  handleValidationErrors
];

/**
 * Validator for POST /tax-rates
 */
const createTaxRateValidator = [
  body('jurisdiction_code')
    .notEmpty()
    .withMessage('jurisdiction_code is required')
    .trim()
    .isLength({ min: 2, max: 20 })
    .withMessage('jurisdiction_code must be between 2 and 20 characters')
    .matches(/^[A-Z0-9-]+$/)
    .withMessage('jurisdiction_code must contain only uppercase letters, numbers, and hyphens'),
  body('tax_type')
    .notEmpty()
    .withMessage('tax_type is required')
    .trim()
    .isLength({ min: 1, max: 50 })
    .withMessage('tax_type must be between 1 and 50 characters')
    .matches(/^[a-z_]+$/)
    .withMessage('tax_type must contain only lowercase letters and underscores'),
  body('rate')
    .notEmpty()
    .withMessage('rate is required')
    .isFloat({ min: 0, max: 1 })
    .withMessage('rate must be a decimal between 0 and 1'),
  body('effective_date')
    .optional()
    .isISO8601()
    .withMessage('effective_date must be a valid ISO 8601 date'),
  body('expiration_date')
    .optional()
    .isISO8601()
    .withMessage('expiration_date must be a valid ISO 8601 date'),
  body('description')
    .optional()
    .trim()
    .isLength({ max: 255 })
    .withMessage('description must be at most 255 characters'),
  body('metadata')
    .optional()
    .isObject()
    .withMessage('metadata must be an object'),
  handleValidationErrors
];

/**
 * Validator for POST /tax-exemptions
 */
const createTaxExemptionValidator = [
  body('customer_id')
    .notEmpty()
    .withMessage('customer_id is required')
    .isInt({ min: 1 })
    .withMessage('customer_id must be a positive integer'),
  body('tax_type')
    .notEmpty()
    .withMessage('tax_type is required')
    .trim()
    .isLength({ min: 1, max: 50 })
    .withMessage('tax_type must be between 1 and 50 characters'),
  body('certificate_number')
    .notEmpty()
    .withMessage('certificate_number is required')
    .trim()
    .isLength({ min: 1, max: 100 })
    .withMessage('certificate_number must be between 1 and 100 characters'),
  handleValidationErrors
];

/**
 * Validator for GET /tax-calculations/invoice/:invoiceId
 */
const getTaxCalculationsForInvoiceValidator = [
  param('invoiceId')
    .isInt({ min: 1 })
    .withMessage('Invoice ID must be a positive integer'),
  handleValidationErrors
];

// ============================================================================
// PRORATION VALIDATORS (PHASE 3)
// ============================================================================

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

// ============================================================================
// USAGE VALIDATORS (Phase 4)
// ============================================================================

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

// ============================================================================
// EXPORTS
// ============================================================================

module.exports = {
  // Customer validators
  getCustomerByIdValidator,
  getCustomerByExternalIdValidator,
  createCustomerValidator,
  setDefaultPaymentMethodValidator,
  updateCustomerValidator,

  // Billing state validator
  getBillingStateValidator,

  // Payment method validators
  listPaymentMethodsValidator,
  addPaymentMethodValidator,
  setDefaultPaymentMethodByIdValidator,
  deletePaymentMethodValidator,

  // Subscription validators
  getSubscriptionByIdValidator,
  listSubscriptionsValidator,
  createSubscriptionValidator,
  cancelSubscriptionValidator,
  updateSubscriptionValidator,

  // Charge validators
  createOneTimeChargeValidator,

  // Refund validators
  createRefundValidator,

  // Tax validators (Phase 1)
  getTaxRatesByJurisdictionValidator,
  createTaxRateValidator,
  createTaxExemptionValidator,
  getTaxCalculationsForInvoiceValidator,

  // Proration validators (Phase 3)
  calculateProrationValidator,
  applySubscriptionChangeValidator,
  calculateCancellationRefundValidator,

  // Usage validators (Phase 4)
  recordUsageValidator,
  calculateUsageChargesValidator,
  getUsageReportValidator,

  // Utility
  handleValidationErrors
};
