// Main validator exports - re-export all validators from domain files
const customerValidators = require('./customerValidators');
const billingStateValidators = require('./billingStateValidators');
const paymentMethodValidators = require('./paymentMethodValidators');
const subscriptionValidators = require('./subscriptionValidators');
const chargeValidators = require('./chargeValidators');
const refundValidators = require('./refundValidators');
const taxValidators = require('./taxValidators');
const prorationValidators = require('./prorationValidators');
const usageValidators = require('./usageValidators');
const invoiceValidators = require('./invoiceValidators');
const { handleValidationErrors } = require('./shared/validationUtils');

module.exports = {
  // Customer validators
  ...customerValidators,

  // Billing state validator
  ...billingStateValidators,

  // Payment method validators
  ...paymentMethodValidators,

  // Subscription validators
  ...subscriptionValidators,

  // Charge validators
  ...chargeValidators,

  // Refund validators
  ...refundValidators,

  // Tax validators (Phase 1)
  ...taxValidators,

  // Proration validators (Phase 3)
  ...prorationValidators,

  // Usage validators (Phase 4)
  ...usageValidators,

  // Invoice validators (Phase 5)
  ...invoiceValidators,

  // Utility
  handleValidationErrors
};