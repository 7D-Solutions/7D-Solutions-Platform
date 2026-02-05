const express = require('express');
const { getTilledClient } = require('../tilledClientFactory');
const CustomerService = require('../services/CustomerService');
const PaymentMethodService = require('../services/PaymentMethodService');
const { requireAppId, rejectSensitiveData } = require('../middleware');
const {
  getCustomerByIdValidator,
  getCustomerByExternalIdValidator,
  createCustomerValidator,
  setDefaultPaymentMethodValidator,
  updateCustomerValidator
} = require('../validators/customerValidators');

const router = express.Router();
const customerService = new CustomerService(getTilledClient);
const paymentMethodService = new PaymentMethodService(getTilledClient, customerService);

// Apply requireAppId middleware to all routes in this file
router.use(requireAppId());

// GET /api/billing/customers/:id
router.get('/:id', getCustomerByIdValidator, async (req, res, next) => {
  try {
    const { id } = req.params;
    const appId = req.verifiedAppId;

    const customer = await customerService.getCustomerById(appId, Number(id));
    res.json(customer);
  } catch (error) {
    next(error);
  }
});

// GET /api/billing/customers (by external_customer_id)
router.get('/', getCustomerByExternalIdValidator, async (req, res, next) => {
  try {
    const { external_customer_id } = req.query;
    const appId = req.verifiedAppId;

    const customer = await customerService.findCustomer(appId, external_customer_id);
    res.json(customer);
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/customers
router.post('/', rejectSensitiveData, createCustomerValidator, async (req, res, next) => {
  try {
    const { email, name, external_customer_id, metadata } = req.body;
    const appId = req.verifiedAppId;

    const customer = await customerService.createCustomer(appId, email, name, external_customer_id, metadata);
    res.status(201).json(customer);
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/customers/:id/default-payment-method
router.post('/:id/default-payment-method', rejectSensitiveData, setDefaultPaymentMethodValidator, async (req, res, next) => {
  try {
    const { id } = req.params;
    const { payment_method_id, payment_method_type } = req.body;
    const appId = req.verifiedAppId;

    const customer = await paymentMethodService.setDefaultPaymentMethod(
      appId,
      Number(id),
      payment_method_id,
      payment_method_type
    );
    res.json(customer);
  } catch (error) {
    next(error);
  }
});

// PUT /api/billing/customers/:id
router.put('/:id', rejectSensitiveData, updateCustomerValidator, async (req, res, next) => {
  try {
    const { id } = req.params;
    const { ...updates } = req.body;
    const appId = req.verifiedAppId;

    const customer = await customerService.updateCustomer(appId, Number(id), updates);
    res.json(customer);
  } catch (error) {
    next(error);
  }
});

module.exports = router;