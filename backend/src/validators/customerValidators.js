const { body, param, query } = require('express-validator');
const {
  handleValidationErrors,
  positiveIntParam,
  enumField
} = require('./shared/validationUtils');

/**
 * Validator for GET /customers/:id
 */
const getCustomerByIdValidator = [
  positiveIntParam('id', 'Customer ID'),
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
  positiveIntParam('id', 'Customer ID'),
  body('payment_method_id')
    .notEmpty()
    .withMessage('payment_method_id is required')
    .trim(),
  enumField('body', 'payment_method_type', ['card', 'ach_debit', 'eft_debit'], { required: true }),
  handleValidationErrors
];

/**
 * Validator for PUT /customers/:id
 */
const updateCustomerValidator = [
  positiveIntParam('id', 'Customer ID'),
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

module.exports = {
  getCustomerByIdValidator,
  getCustomerByExternalIdValidator,
  createCustomerValidator,
  setDefaultPaymentMethodValidator,
  updateCustomerValidator
};
