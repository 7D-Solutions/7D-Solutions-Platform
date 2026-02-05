const { body, param, query } = require('express-validator');
const { handleValidationErrors } = require('./shared/validationErrorHandler');

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

module.exports = {
  listPaymentMethodsValidator,
  addPaymentMethodValidator,
  setDefaultPaymentMethodByIdValidator,
  deletePaymentMethodValidator
};