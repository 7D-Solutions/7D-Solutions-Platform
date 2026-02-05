const express = require('express');
const TaxService = require('../services/TaxService');
const { requireAppId } = require('../middleware');
const {
  getTaxRatesByJurisdictionValidator,
  createTaxRateValidator,
  createTaxExemptionValidator,
  getTaxCalculationsForInvoiceValidator
} = require('../validators/taxValidators');

const router = express.Router();
const taxService = new TaxService();

// Apply requireAppId middleware to all routes in this file
router.use(requireAppId());

// GET /api/billing/tax-rates/:jurisdictionCode
router.get('/rates/:jurisdictionCode', getTaxRatesByJurisdictionValidator, async (req, res, next) => {
  try {
    const { jurisdictionCode } = req.params;
    const appId = req.verifiedAppId;

    const taxRates = await taxService.getTaxRatesByJurisdiction(appId, jurisdictionCode);
    res.json({ tax_rates: taxRates });
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/tax-rates
router.post('/rates', createTaxRateValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const { jurisdiction_code, tax_type, rate, effective_date, expiration_date, description, metadata } = req.body;

    const options = {};
    if (effective_date) options.effectiveDate = new Date(effective_date);
    if (expiration_date) options.expirationDate = new Date(expiration_date);
    if (description) options.description = description;
    if (metadata) options.metadata = metadata;

    const taxRate = await taxService.createTaxRate(
      appId,
      jurisdiction_code,
      tax_type,
      parseFloat(rate),
      options
    );

    res.status(201).json({ tax_rate: taxRate });
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/tax-exemptions
router.post('/exemptions', createTaxExemptionValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const { customer_id, tax_type, certificate_number } = req.body;

    const exemption = await taxService.createTaxExemption(
      appId,
      Number(customer_id),
      tax_type,
      certificate_number
    );

    res.status(201).json({ tax_exemption: exemption });
  } catch (error) {
    next(error);
  }
});

// GET /api/billing/tax-calculations/invoice/:invoiceId
router.get('/calculations/invoice/:invoiceId', getTaxCalculationsForInvoiceValidator, async (req, res, next) => {
  try {
    const { invoiceId } = req.params;
    const appId = req.verifiedAppId;

    const taxCalculations = await taxService.getTaxCalculationsForInvoice(appId, Number(invoiceId));
    res.json({ tax_calculations: taxCalculations });
  } catch (error) {
    next(error);
  }
});

module.exports = router;