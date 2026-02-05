const { query } = require('express-validator');
const { handleValidationErrors } = require('./shared/validationErrorHandler');

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

module.exports = {
  getBillingStateValidator
};