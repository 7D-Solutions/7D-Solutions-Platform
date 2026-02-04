const express = require('express');
const BillingService = require('../billingService');
const { requireAppId, rejectSensitiveData } = require('../middleware');
const {
  createInvoiceValidator,
  getInvoiceValidator,
  addInvoiceLineItemValidator,
  generateInvoiceFromSubscriptionValidator,
  updateInvoiceStatusValidator,
  listInvoicesValidator
} = require('../validators/invoiceValidators');

const router = express.Router();
const billingService = new BillingService();

// Apply requireAppId middleware to all routes in this file
router.use(requireAppId());

/**
 * POST /invoices
 * Create a new invoice
 *
 * Body:
 *   {
 *     customer_id: integer,
 *     amount_cents: integer,
 *     subscription_id: integer (optional),
 *     status: string (optional, default: 'draft'),
 *     currency: string (optional, default: 'usd'),
 *     due_at: string (ISO 8601, optional),
 *     metadata: object (optional),
 *     billing_period_start: string (ISO 8601, optional),
 *     billing_period_end: string (ISO 8601, optional),
 *     line_item_details: object (optional),
 *     compliance_codes: object (optional)
 *   }
 *
 * Response:
 *   {
 *     invoice: { ... } // Created invoice from InvoiceService.createInvoice
 *   }
 */
router.post('/', rejectSensitiveData, createInvoiceValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const {
      customer_id,
      amount_cents,
      subscription_id = null,
      status = 'draft',
      currency = 'usd',
      due_at = null,
      metadata = {},
      billing_period_start = null,
      billing_period_end = null,
      line_item_details = null,
      compliance_codes = null
    } = req.body;

    const invoice = await billingService.createInvoice({
      appId,
      customerId: customer_id,
      subscriptionId: subscription_id,
      status,
      amountCents: amount_cents,
      currency,
      dueAt: due_at ? new Date(due_at) : null,
      metadata,
      billingPeriodStart: billing_period_start ? new Date(billing_period_start) : null,
      billingPeriodEnd: billing_period_end ? new Date(billing_period_end) : null,
      lineItemDetails: line_item_details,
      complianceCodes: compliance_codes
    });

    res.status(201).json(invoice);
  } catch (error) {
    next(error);
  }
});

/**
 * GET /invoices/:id
 * Get invoice by ID
 *
 * Query parameters:
 *   include_line_items: boolean (optional, default: false)
 *
 * Response:
 *   {
 *     invoice: { ... } // Invoice details from InvoiceService.getInvoice
 *   }
 */
router.get('/:id', rejectSensitiveData, getInvoiceValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const { id } = req.params;
    const includeLineItems = req.query.include_line_items === 'true' || req.query.include_line_items === true;

    const invoice = await billingService.getInvoice(
      appId,
      Number(id),
      includeLineItems
    );

    res.json(invoice);
  } catch (error) {
    next(error);
  }
});

/**
 * POST /invoices/:id/line-items
 * Add line item to invoice
 *
 * Body:
 *   {
 *     line_item_type: string ('subscription', 'usage', 'tax', 'discount', 'fee', 'other'),
 *     description: string,
 *     quantity: number (decimal),
 *     unit_price_cents: integer,
 *     metadata: object (optional)
 *   }
 *
 * Response:
 *   {
 *     line_item: { ... } // Created line item from InvoiceService.addInvoiceLineItem
 *   }
 */
router.post('/:id/line-items', rejectSensitiveData, addInvoiceLineItemValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const { id } = req.params;
    const {
      line_item_type,
      description,
      quantity,
      unit_price_cents,
      metadata = {}
    } = req.body;

    const lineItem = await billingService.addInvoiceLineItem({
      appId,
      invoiceId: Number(id),
      lineItemType: line_item_type,
      description,
      quantity: Number(quantity),
      unitPriceCents: unit_price_cents,
      metadata
    });

    res.status(201).json(lineItem);
  } catch (error) {
    next(error);
  }
});

/**
 * POST /invoices/generate-from-subscription
 * Generate invoice from subscription for a billing period
 *
 * Body:
 *   {
 *     subscription_id: integer,
 *     billing_period_start: string (ISO 8601),
 *     billing_period_end: string (ISO 8601),
 *     include_usage: boolean (optional, default: true),
 *     include_tax: boolean (optional, default: true),
 *     include_discounts: boolean (optional, default: true)
 *   }
 *
 * Response:
 *   {
 *     invoice: { ... } // Generated invoice from InvoiceService.generateInvoiceFromSubscription
 *   }
 */
router.post('/generate-from-subscription', rejectSensitiveData, generateInvoiceFromSubscriptionValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const {
      subscription_id,
      billing_period_start,
      billing_period_end,
      include_usage = true,
      include_tax = true,
      include_discounts = true
    } = req.body;

    const invoice = await billingService.generateInvoiceFromSubscription({
      appId,
      subscriptionId: Number(subscription_id),
      billingPeriodStart: new Date(billing_period_start),
      billingPeriodEnd: new Date(billing_period_end),
      includeUsage: include_usage,
      includeTax: include_tax,
      includeDiscounts: include_discounts
    });

    res.status(201).json(invoice);
  } catch (error) {
    next(error);
  }
});

/**
 * PATCH /invoices/:id/status
 * Update invoice status
 *
 * Body:
 *   {
 *     status: string ('draft', 'open', 'paid', 'void', 'uncollectible'),
 *     paid_at: string (ISO 8601, optional) - required when marking as paid
 *   }
 *
 * Response:
 *   {
 *     invoice: { ... } // Updated invoice from InvoiceService.updateInvoiceStatus
 *   }
 */
router.patch('/:id/status', rejectSensitiveData, updateInvoiceStatusValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const { id } = req.params;
    const { status, paid_at = null } = req.body;

    const invoice = await billingService.updateInvoiceStatus(
      appId,
      Number(id),
      status,
      { paidAt: paid_at ? new Date(paid_at) : null }
    );

    res.json(invoice);
  } catch (error) {
    next(error);
  }
});

/**
 * GET /invoices
 * List invoices with filters
 *
 * Query parameters:
 *   customer_id: integer (optional),
 *   subscription_id: integer (optional),
 *   status: string (optional),
 *   start_date: string (ISO 8601, optional),
 *   end_date: string (ISO 8601, optional),
 *   limit: integer (optional, default: 50, max: 100),
 *   offset: integer (optional, default: 0)
 *
 * Response:
 *   {
 *     invoices: [ ... ], // Array of invoices
 *     pagination: { total, limit, offset, has_more }
 *   }
 */
router.get('/', rejectSensitiveData, listInvoicesValidator, async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const {
      customer_id = null,
      subscription_id = null,
      status = null,
      start_date = null,
      end_date = null,
      limit = 50,
      offset = 0
    } = req.query;

    const filters = {
      appId,
      customerId: customer_id ? Number(customer_id) : null,
      subscriptionId: subscription_id ? Number(subscription_id) : null,
      status,
      startDate: start_date ? new Date(start_date) : null,
      endDate: end_date ? new Date(end_date) : null,
      limit: Number(limit),
      offset: Number(offset)
    };

    const result = await billingService.listInvoices(filters);

    res.json({
      invoices: result.invoices,
      pagination: result.pagination
    });
  } catch (error) {
    next(error);
  }
});

module.exports = router;