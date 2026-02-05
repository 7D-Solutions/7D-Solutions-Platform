const express = require('express');
const { getTilledClient } = require('../tilledClientFactory');
const CustomerService = require('../services/CustomerService');
const PaymentMethodService = require('../services/PaymentMethodService');
const { requireAppId, rejectSensitiveData } = require('../middleware');
const {
  listPaymentMethodsValidator,
  addPaymentMethodValidator,
  setDefaultPaymentMethodByIdValidator,
  deletePaymentMethodValidator
} = require('../validators/paymentMethodValidators');

const router = express.Router();
const customerService = new CustomerService(getTilledClient);
const paymentMethodService = new PaymentMethodService(getTilledClient, customerService);

// Apply requireAppId middleware to all routes in this file
router.use(requireAppId());

// GET /api/billing/payment-methods (list for customer)
router.get('/', listPaymentMethodsValidator, async (req, res, next) => {
  try {
    const { billing_customer_id } = req.query;
    const appId = req.verifiedAppId;

    const result = await paymentMethodService.listPaymentMethods(appId, Number(billing_customer_id));
    res.json(result);
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/payment-methods (add payment method)
router.post('/', rejectSensitiveData, addPaymentMethodValidator, async (req, res, next) => {
  try {
    const { billing_customer_id, payment_method_id } = req.body;
    const appId = req.verifiedAppId;

    const paymentMethod = await paymentMethodService.addPaymentMethod(
      appId,
      Number(billing_customer_id),
      payment_method_id
    );
    res.status(201).json(paymentMethod);
  } catch (error) {
    next(error);
  }
});

// PUT /api/billing/payment-methods/:id/default (set default payment method)
router.put('/:id/default', rejectSensitiveData, setDefaultPaymentMethodByIdValidator, async (req, res, next) => {
  try {
    const { id: tilledPaymentMethodId } = req.params;
    const { billing_customer_id } = req.body;
    const appId = req.verifiedAppId;

    const paymentMethod = await paymentMethodService.setDefaultPaymentMethodById(
      appId,
      Number(billing_customer_id),
      tilledPaymentMethodId
    );
    res.json(paymentMethod);
  } catch (error) {
    next(error);
  }
});

// DELETE /api/billing/payment-methods/:id (soft delete payment method)
router.delete('/:id', deletePaymentMethodValidator, async (req, res, next) => {
  try {
    const { id: tilledPaymentMethodId } = req.params;
    const { billing_customer_id } = req.query;
    const appId = req.verifiedAppId;

    const result = await paymentMethodService.deletePaymentMethod(
      appId,
      Number(billing_customer_id),
      tilledPaymentMethodId
    );
    res.json(result);
  } catch (error) {
    next(error);
  }
});

module.exports = router;