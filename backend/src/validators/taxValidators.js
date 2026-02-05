const { body, param } = require('express-validator');
const { handleValidationErrors } = require('./shared/validationErrorHandler');
const {
  positiveIntParam,
  positiveIntBody,
  isoDateField
} = require('./shared/validationUtils');

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
  isoDateField('body', 'effective_date'),
  isoDateField('body', 'expiration_date'),
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
  positiveIntBody('customer_id'),
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
  positiveIntParam('invoiceId', 'Invoice ID'),
  handleValidationErrors
];

module.exports = {
  getTaxRatesByJurisdictionValidator,
  createTaxRateValidator,
  createTaxExemptionValidator,
  getTaxCalculationsForInvoiceValidator
};
