const crypto = require('crypto');
const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError } = require('../utils/errors');

/**
 * InvoiceService - Phase 5: Invoice Customization
 *
 * Generic invoice service for invoice management and customization:
 * - Create and manage invoices
 * - Add detailed line items
 * - Generate invoices from subscriptions and usage
 * - Customize with generic fields (billing periods, line item details, compliance codes)
 *
 * Uses billing_invoices and billing_invoice_line_items tables.
 * Follows generic billing module pattern (not industry-specific).
 *
 * @author MistyBridge (WhiteBadger)
 * @phase 5
 */
class InvoiceService {
  constructor(getTilledClientFn) {
    this.getTilledClient = getTilledClientFn;
  }

  /**
   * Create a new invoice
   * @param {Object} params
   * @param {string} params.appId - Application identifier
   * @param {number} params.customerId - Billing customer ID
   * @param {number} params.subscriptionId - Optional subscription ID
   * @param {string} params.status - Invoice status ('draft', 'open', 'paid', 'void')
   * @param {number} params.amountCents - Invoice total in cents
   * @param {string} params.currency - Currency code (default: 'usd')
   * @param {Date} params.dueAt - Optional due date
   * @param {Object} params.metadata - Additional invoice metadata
   * @param {Date} params.billingPeriodStart - Optional billing period start
   * @param {Date} params.billingPeriodEnd - Optional billing period end
   * @param {Object} params.lineItemDetails - Optional line item details JSON
   * @param {Object} params.complianceCodes - Optional compliance codes JSON
   * @returns {Promise<Object>} Created invoice
   */
  async createInvoice(params) {
    const {
      appId,
      customerId,
      subscriptionId = null,
      status = 'draft',
      amountCents,
      currency = 'usd',
      dueAt = null,
      metadata = {},
      billingPeriodStart = null,
      billingPeriodEnd = null,
      lineItemDetails = null,
      complianceCodes = null
    } = params;

    // Validate required fields
    if (!appId || !customerId || !amountCents) {
      throw new ValidationError('appId, customerId, and amountCents are required');
    }

    if (amountCents < 0) {
      throw new ValidationError('amountCents must be non-negative');
    }

    // Validate status
    const validStatuses = ['draft', 'open', 'paid', 'void', 'uncollectible'];
    if (!validStatuses.includes(status)) {
      throw new ValidationError(`status must be one of: ${validStatuses.join(', ')}`);
    }

    // Verify customer exists
    const customer = await billingPrisma.billing_customers.findFirst({
      where: {
        id: customerId,
        app_id: appId
      }
    });

    if (!customer) {
      throw new NotFoundError(`Customer ${customerId} not found for app ${appId}`);
    }

    // Verify subscription exists if provided
    if (subscriptionId) {
      const subscription = await billingPrisma.billing_subscriptions.findFirst({
        where: {
          id: subscriptionId,
          app_id: appId,
          billing_customer_id: customerId
        }
      });

      if (!subscription) {
        throw new NotFoundError(`Subscription ${subscriptionId} not found for customer ${customerId} and app ${appId}`);
      }
    }

    // Generate unique invoice ID for Tilled (required field)
    const tilledInvoiceId = `in_${appId}_${Date.now()}_${crypto.randomBytes(8).toString('hex')}`;

    // Create invoice
    const invoice = await billingPrisma.billing_invoices.create({
      data: {
        app_id: appId,
        tilled_invoice_id: tilledInvoiceId,
        billing_customer_id: customerId,
        subscription_id: subscriptionId,
        status,
        amount_cents: amountCents,
        currency,
        due_at: dueAt,
        metadata,
        billing_period_start: billingPeriodStart,
        billing_period_end: billingPeriodEnd,
        line_item_details: lineItemDetails,
        compliance_codes: complianceCodes
      }
    });

    logger.info({
      message: 'Invoice created',
      appId,
      customerId,
      subscriptionId,
      invoiceId: invoice.id,
      amountCents,
      status
    });

    return {
      id: invoice.id,
      tilled_invoice_id: invoice.tilled_invoice_id,
      app_id: invoice.app_id,
      billing_customer_id: invoice.billing_customer_id,
      subscription_id: invoice.subscription_id,
      status: invoice.status,
      amount_cents: invoice.amount_cents,
      currency: invoice.currency,
      due_at: invoice.due_at,
      metadata: invoice.metadata,
      billing_period_start: invoice.billing_period_start,
      billing_period_end: invoice.billing_period_end,
      line_item_details: invoice.line_item_details,
      compliance_codes: invoice.compliance_codes,
      created_at: invoice.created_at,
      updated_at: invoice.updated_at
    };
  }

  /**
   * Get invoice by ID
   * @param {string} appId - Application identifier
   * @param {number} invoiceId - Invoice ID
   * @param {boolean} includeLineItems - Include line items (default: false)
   * @returns {Promise<Object>} Invoice details
   */
  async getInvoice(appId, invoiceId, includeLineItems = false) {
    if (!appId || !invoiceId) {
      throw new ValidationError('appId and invoiceId are required');
    }

    const invoice = await billingPrisma.billing_invoices.findFirst({
      where: {
        id: invoiceId,
        app_id: appId
      },
      include: {
        customer: {
          select: {
            email: true,
            name: true,
            external_customer_id: true
          }
        },
        subscription: {
          select: {
            plan_name: true,
            plan_id: true,
            status: true
          }
        },
        ...(includeLineItems ? {
          billing_invoice_line_items: true
        } : {})
      }
    });

    if (!invoice) {
      throw new NotFoundError(`Invoice ${invoiceId} not found for app ${appId}`);
    }

    return invoice;
  }

  /**
   * Add line item to invoice
   * @param {Object} params
   * @param {string} params.appId - Application identifier
   * @param {number} params.invoiceId - Invoice ID
   * @param {string} params.lineItemType - Type: 'subscription', 'usage', 'tax', 'discount', 'fee', 'other'
   * @param {string} params.description - Line item description
   * @param {number} params.quantity - Quantity (decimal supported)
   * @param {number} params.unitPriceCents - Price per unit in cents
   * @param {Object} params.metadata - Optional metadata (product/service details)
   * @returns {Promise<Object>} Created line item
   */
  async addInvoiceLineItem(params) {
    const {
      appId,
      invoiceId,
      lineItemType,
      description,
      quantity,
      unitPriceCents,
      metadata = {}
    } = params;

    // Validate required fields
    if (!appId || !invoiceId || !lineItemType || !description || quantity === undefined || unitPriceCents === undefined) {
      throw new ValidationError('appId, invoiceId, lineItemType, description, quantity, and unitPriceCents are required');
    }

    if (quantity < 0) {
      throw new ValidationError('quantity must be non-negative');
    }

    if (unitPriceCents < 0) {
      throw new ValidationError('unitPriceCents must be non-negative');
    }

    // Validate line item type
    const validTypes = ['subscription', 'usage', 'tax', 'discount', 'fee', 'other'];
    if (!validTypes.includes(lineItemType)) {
      throw new ValidationError(`lineItemType must be one of: ${validTypes.join(', ')}`);
    }

    // Verify invoice exists and belongs to app
    const invoice = await billingPrisma.billing_invoices.findFirst({
      where: {
        id: invoiceId,
        app_id: appId
      }
    });

    if (!invoice) {
      throw new NotFoundError(`Invoice ${invoiceId} not found for app ${appId}`);
    }

    // Calculate amount
    const amountCents = Math.round(Number(quantity) * unitPriceCents);

    // Create line item
    const lineItem = await billingPrisma.billing_invoice_line_items.create({
      data: {
        app_id: appId,
        invoice_id: invoiceId,
        line_item_type: lineItemType,
        description,
        quantity,
        unit_price_cents: unitPriceCents,
        amount_cents: amountCents,
        metadata
      }
    });

    // Update invoice total with sum of all line items
    const lineItemsSum = await billingPrisma.billing_invoice_line_items.aggregate({
      where: { invoice_id: invoiceId },
      _sum: { amount_cents: true }
    });
    await billingPrisma.billing_invoices.update({
      where: { id: invoiceId },
      data: { amount_cents: lineItemsSum._sum.amount_cents || 0 }
    });

    logger.info({
      message: 'Invoice line item added',
      appId,
      invoiceId,
      lineItemId: lineItem.id,
      lineItemType,
      amountCents
    });

    return {
      id: lineItem.id,
      app_id: lineItem.app_id,
      invoice_id: lineItem.invoice_id,
      line_item_type: lineItem.line_item_type,
      description: lineItem.description,
      quantity: Number(lineItem.quantity),
      unit_price_cents: lineItem.unit_price_cents,
      amount_cents: lineItem.amount_cents,
      metadata: lineItem.metadata,
      created_at: lineItem.created_at
    };
  }

  /**
   * Generate invoice from subscription for a billing period
   * @param {Object} params
   * @param {string} params.appId - Application identifier
   * @param {number} params.subscriptionId - Subscription ID
   * @param {Date} params.billingPeriodStart - Billing period start
   * @param {Date} params.billingPeriodEnd - Billing period end
   * @param {boolean} params.includeUsage - Include metered usage charges (default: true)
   * @param {boolean} params.includeTax - Include tax calculations (default: true)
   * @param {boolean} params.includeDiscounts - Include discounts (default: true)
   * @returns {Promise<Object>} Generated invoice with line items
   */
  async generateInvoiceFromSubscription(params) {
    const {
      appId,
      subscriptionId,
      billingPeriodStart,
      billingPeriodEnd,
      includeUsage = true,
      includeTax = true,
      includeDiscounts = true
    } = params;

    // Validate required fields
    if (!appId || !subscriptionId || !billingPeriodStart || !billingPeriodEnd) {
      throw new ValidationError('appId, subscriptionId, billingPeriodStart, and billingPeriodEnd are required');
    }

    if (billingPeriodStart >= billingPeriodEnd) {
      throw new ValidationError('billingPeriodStart must be before billingPeriodEnd');
    }

    // Verify subscription exists
    const subscription = await billingPrisma.billing_subscriptions.findFirst({
      where: {
        id: subscriptionId,
        app_id: appId
      },
      include: {
        billing_customers: true
      }
    });

    if (!subscription) {
      throw new NotFoundError(`Subscription ${subscriptionId} not found for app ${appId}`);
    }

    // Get subscription price
    const subscriptionAmountCents = subscription.price_cents;

    // Start building invoice
    let totalAmountCents = subscriptionAmountCents;
    const lineItems = [];

    // Add subscription line item
    lineItems.push({
      line_item_type: 'subscription',
      description: `Subscription: ${subscription.plan_name}`,
      quantity: 1,
      unit_price_cents: subscriptionAmountCents,
      amount_cents: subscriptionAmountCents,
      metadata: {
        plan_id: subscription.plan_id,
        plan_name: subscription.plan_name,
        billing_period_start: billingPeriodStart,
        billing_period_end: billingPeriodEnd
      }
    });

    // TODO: Add usage charges if includeUsage is true
    // This would integrate with UsageService.calculateUsageCharges()

    // TODO: Add discounts if includeDiscounts is true
    // This would integrate with DiscountService.calculateDiscounts()

    // TODO: Add tax if includeTax is true
    // This would integrate with TaxService.calculateTax()

    // For now, create invoice with subscription line item only
    // Actual implementation would calculate all components

    // Create invoice
    const invoice = await this.createInvoice({
      appId,
      customerId: subscription.billing_customer_id,
      subscriptionId,
      status: 'draft',
      amountCents: totalAmountCents,
      billingPeriodStart,
      billingPeriodEnd,
      lineItemDetails: {
        generated_from: 'subscription',
        period: { start: billingPeriodStart, end: billingPeriodEnd },
        components: {
          subscription: true,
          usage: includeUsage,
          discounts: includeDiscounts,
          tax: includeTax
        }
      }
    });

    // Add line items
    for (const lineItem of lineItems) {
      await this.addInvoiceLineItem({
        appId,
        invoiceId: invoice.id,
        lineItemType: lineItem.line_item_type,
        description: lineItem.description,
        quantity: lineItem.quantity,
        unitPriceCents: lineItem.unit_price_cents,
        metadata: lineItem.metadata || {}
      });
    }

    // Get invoice with line items
    const fullInvoice = await this.getInvoice(appId, invoice.id, true);

    logger.info({
      message: 'Invoice generated from subscription',
      appId,
      subscriptionId,
      invoiceId: invoice.id,
      totalAmountCents,
      lineItemCount: lineItems.length
    });

    return fullInvoice;
  }

  /**
   * Update invoice status (e.g., mark as paid)
   * @param {string} appId - Application identifier
   * @param {number} invoiceId - Invoice ID
   * @param {string} status - New status
   * @param {Object} options - Optional parameters
   * @param {Date} options.paidAt - Paid date (if marking as paid)
   * @returns {Promise<Object>} Updated invoice
   */
  async updateInvoiceStatus(appId, invoiceId, status, options = {}) {
    if (!appId || !invoiceId || !status) {
      throw new ValidationError('appId, invoiceId, and status are required');
    }

    const validStatuses = ['draft', 'open', 'paid', 'void', 'uncollectible'];
    if (!validStatuses.includes(status)) {
      throw new ValidationError(`status must be one of: ${validStatuses.join(', ')}`);
    }

    // Verify invoice exists
    const invoice = await billingPrisma.billing_invoices.findFirst({
      where: {
        id: invoiceId,
        app_id: appId
      }
    });

    if (!invoice) {
      throw new NotFoundError(`Invoice ${invoiceId} not found for app ${appId}`);
    }

    // Prepare update data
    const updateData = {
      status,
      updated_at: new Date()
    };

    if (status === 'paid' && options.paidAt) {
      updateData.paid_at = options.paidAt;
    } else if (status === 'paid' && !invoice.paid_at) {
      updateData.paid_at = new Date();
    }

    // Update invoice
    const updatedInvoice = await billingPrisma.billing_invoices.update({
      where: { id: invoiceId },
      data: updateData
    });

    logger.info({
      message: 'Invoice status updated',
      appId,
      invoiceId,
      oldStatus: invoice.status,
      newStatus: status,
      paidAt: updatedInvoice.paid_at
    });

    return updatedInvoice;
  }

  /**
   * List invoices for customer or subscription
   * @param {Object} filters
   * @param {string} filters.appId - Application identifier
   * @param {number} filters.customerId - Optional customer ID filter
   * @param {number} filters.subscriptionId - Optional subscription ID filter
   * @param {string} filters.status - Optional status filter
   * @param {Date} filters.startDate - Optional start date for created_at
   * @param {Date} filters.endDate - Optional end date for created_at
   * @param {number} filters.limit - Maximum results (default: 50)
   * @param {number} filters.offset - Pagination offset (default: 0)
   * @returns {Promise<Object>} List of invoices with pagination info
   */
  async listInvoices(filters = {}) {
    const {
      appId,
      customerId = null,
      subscriptionId = null,
      status = null,
      startDate = null,
      endDate = null,
      limit = 50,
      offset = 0
    } = filters;

    if (!appId) {
      throw new ValidationError('appId is required');
    }

    // Build where clause
    const where = {
      app_id: appId
    };

    if (customerId) {
      where.billing_customer_id = customerId;
    }

    if (subscriptionId) {
      where.subscription_id = subscriptionId;
    }

    if (status) {
      where.status = status;
    }

    if (startDate || endDate) {
      where.created_at = {};
      if (startDate) {
        where.created_at.gte = startDate;
      }
      if (endDate) {
        where.created_at.lte = endDate;
      }
    }

    // Get total count
    const total = await billingPrisma.billing_invoices.count({ where });

    // Get invoices
    const invoices = await billingPrisma.billing_invoices.findMany({
      where,
      include: {
        customer: {
          select: {
            email: true,
            name: true
          }
        },
        subscription: {
          select: {
            plan_name: true
          }
        }
      },
      orderBy: { created_at: 'desc' },
      take: limit,
      skip: offset
    });

    return {
      invoices,
      pagination: {
        total,
        limit,
        offset,
        hasMore: offset + invoices.length < total
      }
    };
  }
}

module.exports = InvoiceService;