const { query } = require('express-validator');
const {
  dateRangeFields,
  isoDateField,
  positiveIntQuery,
  enumField,
  paginationQuery
} = require('./shared/validationUtils');
const { handleValidationErrors } = require('./shared/validationErrorHandler');

/**
 * Validator for GET /reports/revenue
 * Query parameters:
 * - start_date: ISO 8601 date (required)
 * - end_date: ISO 8601 date (required)
 * - granularity: 'daily', 'weekly', 'monthly', 'quarterly' (optional, default: 'daily')
 * - customer_id: positive integer (optional)
 * - subscription_id: positive integer (optional)
 * - charge_type: 'subscription', 'one_time', 'usage' (optional)
 */
const getRevenueReportValidator = [
  ...dateRangeFields('query', 'start_date', 'end_date', { required: true }),
  enumField('query', 'granularity', ['daily', 'weekly', 'monthly', 'quarterly'], {
    required: false,
    label: 'granularity'
  }),
  positiveIntQuery('customer_id', { required: false, label: 'customer_id' }),
  positiveIntQuery('subscription_id', { required: false, label: 'subscription_id' }),
  enumField('query', 'charge_type', ['subscription', 'one_time', 'usage'], {
    required: false,
    label: 'charge_type'
  }),
  ...paginationQuery({ maxLimit: 1000 }), // Higher limit for reports
  handleValidationErrors
];

/**
 * Validator for GET /reports/mrr
 * Query parameters:
 * - as_of_date: ISO 8601 date (required)
 * - plan_id: string (optional)
 * - include_breakdown: boolean (optional, default: true)
 */
const getMRRReportValidator = [
  isoDateField('query', 'as_of_date', { required: true, label: 'as_of_date' }),
  query('plan_id')
    .optional()
    .isString()
    .withMessage('plan_id must be a string')
    .trim()
    .notEmpty()
    .withMessage('plan_id cannot be empty if provided'),
  query('include_breakdown')
    .optional()
    .isBoolean()
    .withMessage('include_breakdown must be a boolean')
    .toBoolean(),
  handleValidationErrors
];

/**
 * Validator for GET /reports/churn
 * Query parameters:
 * - start_date: ISO 8601 date (required)
 * - end_date: ISO 8601 date (required)
 * - cohort_period: 'daily', 'weekly', 'monthly', 'quarterly' (optional, default: 'monthly')
 * - plan_id: string (optional)
 */
const getChurnReportValidator = [
  ...dateRangeFields('query', 'start_date', 'end_date', { required: true }),
  enumField('query', 'cohort_period', ['daily', 'weekly', 'monthly', 'quarterly'], {
    required: false,
    label: 'cohort_period'
  }),
  query('plan_id')
    .optional()
    .isString()
    .withMessage('plan_id must be a string')
    .trim()
    .notEmpty()
    .withMessage('plan_id cannot be empty if provided'),
  handleValidationErrors
];

/**
 * Validator for GET /reports/aging-receivables
 * Query parameters:
 * - as_of_date: ISO 8601 date (required)
 * - customer_id: positive integer (optional)
 */
const getAgingReceivablesReportValidator = [
  isoDateField('query', 'as_of_date', { required: true, label: 'as_of_date' }),
  positiveIntQuery('customer_id', { required: false, label: 'customer_id' }),
  handleValidationErrors
];

module.exports = {
  getRevenueReportValidator,
  getMRRReportValidator,
  getChurnReportValidator,
  getAgingReceivablesReportValidator
};