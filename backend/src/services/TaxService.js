const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError } = require('../utils/errors');

class TaxService {
  /**
   * Calculate tax for a given amount and customer
   * @param {string} appId - Application identifier
   * @param {number} customerId - Customer ID
   * @param {number} subtotalCents - Amount to calculate tax on (after discounts)
   * @param {Object} options - Optional parameters
   * @param {boolean} options.taxExempt - Whether customer is tax exempt
   * @param {string} options.jurisdictionCode - Override jurisdiction (optional)
   * @returns {Promise<Object>} Tax calculation result
   */
  async calculateTax(appId, customerId, subtotalCents, options = {}) {
    // Validate inputs
    if (!appId || !customerId) {
      throw new ValidationError('appId and customerId are required');
    }

    if (typeof subtotalCents !== 'number' || subtotalCents < 0) {
      throw new ValidationError('subtotalCents must be a non-negative number');
    }

    // If tax exempt, return zero tax
    if (options.taxExempt) {
      return {
        taxAmountCents: 0,
        taxRate: 0,
        jurisdictionCode: options.jurisdictionCode || 'EXEMPT',
        taxType: 'exempt',
        breakdown: []
      };
    }

    // Get customer to determine jurisdiction
    const customer = await billingPrisma.billing_customers.findFirst({
      where: {
        id: customerId,
        app_id: appId
      }
    });

    if (!customer) {
      throw new NotFoundError(`Customer ${customerId} not found for app ${appId}`);
    }

    // Determine jurisdiction code
    let jurisdictionCode = options.jurisdictionCode;

    if (!jurisdictionCode) {
      // Extract jurisdiction from customer metadata or default
      jurisdictionCode = customer.metadata?.jurisdiction_code ||
                        customer.metadata?.state ||
                        'DEFAULT';
    }

    // Get active tax rates for jurisdiction
    const now = new Date();
    const taxRates = await billingPrisma.billing_tax_rates.findMany({
      where: {
        app_id: appId,
        jurisdiction_code: jurisdictionCode,
        effective_date: {
          lte: now
        },
        OR: [
          { expiration_date: null },
          { expiration_date: { gt: now } }
        ]
      },
      orderBy: {
        tax_type: 'asc'
      }
    });

    if (taxRates.length === 0) {
      logger.warn('No tax rates found for jurisdiction', {
        app_id: appId,
        jurisdiction_code: jurisdictionCode,
        customer_id: customerId
      });

      return {
        taxAmountCents: 0,
        taxRate: 0,
        jurisdictionCode,
        taxType: 'none',
        breakdown: []
      };
    }

    // Calculate tax for each rate
    const breakdown = [];
    let totalTaxCents = 0;

    for (const taxRate of taxRates) {
      const rate = parseFloat(taxRate.rate);
      const taxAmountCents = Math.round(subtotalCents * rate);

      totalTaxCents += taxAmountCents;

      breakdown.push({
        taxRateId: taxRate.id,
        taxType: taxRate.tax_type,
        rate: rate,
        taxableAmountCents: subtotalCents,
        taxAmountCents: taxAmountCents,
        description: taxRate.description
      });
    }

    // Calculate combined rate
    const combinedRate = totalTaxCents / subtotalCents;

    return {
      taxAmountCents: totalTaxCents,
      taxRate: combinedRate,
      jurisdictionCode,
      taxType: breakdown.map(b => b.taxType).join(', '),
      breakdown
    };
  }

  /**
   * Get tax rates for a jurisdiction
   * @param {string} appId - Application identifier
   * @param {string} jurisdictionCode - Jurisdiction code (e.g., "CA", "NY-NYC")
   * @returns {Promise<Array>} Array of tax rates
   */
  async getTaxRatesByJurisdiction(appId, jurisdictionCode) {
    if (!appId || !jurisdictionCode) {
      throw new ValidationError('appId and jurisdictionCode are required');
    }

    const now = new Date();
    const taxRates = await billingPrisma.billing_tax_rates.findMany({
      where: {
        app_id: appId,
        jurisdiction_code: jurisdictionCode,
        effective_date: {
          lte: now
        },
        OR: [
          { expiration_date: null },
          { expiration_date: { gt: now } }
        ]
      },
      orderBy: {
        tax_type: 'asc'
      }
    });

    return taxRates.map(rate => ({
      id: rate.id,
      jurisdictionCode: rate.jurisdiction_code,
      taxType: rate.tax_type,
      rate: parseFloat(rate.rate),
      effectiveDate: rate.effective_date,
      expirationDate: rate.expiration_date,
      description: rate.description,
      metadata: rate.metadata
    }));
  }

  /**
   * Create or update a tax rate
   * @param {string} appId - Application identifier
   * @param {string} jurisdictionCode - Jurisdiction code
   * @param {string} taxType - Type of tax
   * @param {number} rate - Tax rate (e.g., 0.0825 for 8.25%)
   * @param {Object} options - Optional parameters
   * @returns {Promise<Object>} Created tax rate
   */
  async createTaxRate(appId, jurisdictionCode, taxType, rate, options = {}) {
    if (!appId || !jurisdictionCode || !taxType) {
      throw new ValidationError('appId, jurisdictionCode, and taxType are required');
    }

    if (typeof rate !== 'number' || rate < 0 || rate > 1) {
      throw new ValidationError('rate must be a number between 0 and 1');
    }

    const effectiveDate = options.effectiveDate || new Date();

    const taxRate = await billingPrisma.billing_tax_rates.create({
      data: {
        app_id: appId,
        jurisdiction_code: jurisdictionCode,
        tax_type: taxType,
        rate: rate,
        effective_date: effectiveDate,
        expiration_date: options.expirationDate || null,
        description: options.description || null,
        metadata: options.metadata || null
      }
    });

    logger.info('Tax rate created', {
      app_id: appId,
      tax_rate_id: taxRate.id,
      jurisdiction_code: jurisdictionCode,
      tax_type: taxType,
      rate
    });

    return taxRate;
  }

  /**
   * Create tax exemption for a customer
   * @param {string} appId - Application identifier
   * @param {number} customerId - Customer ID
   * @param {string} taxType - Type of tax exemption
   * @param {string} certificateNumber - Tax exemption certificate number
   * @returns {Promise<Object>} Created exemption record
   */
  async createTaxExemption(appId, customerId, taxType, certificateNumber) {
    if (!appId || !customerId || !taxType || !certificateNumber) {
      throw new ValidationError('appId, customerId, taxType, and certificateNumber are required');
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

    // Store exemption in customer metadata
    const currentMetadata = customer.metadata || {};
    const taxExemptions = currentMetadata.tax_exemptions || [];

    // Check if exemption already exists
    const existingExemption = taxExemptions.find(
      e => e.tax_type === taxType && e.certificate_number === certificateNumber
    );

    if (existingExemption) {
      throw new ValidationError(`Tax exemption already exists for ${taxType}`);
    }

    // Add new exemption
    const newExemption = {
      tax_type: taxType,
      certificate_number: certificateNumber,
      status: 'active',
      created_at: new Date().toISOString()
    };

    taxExemptions.push(newExemption);

    // Update customer metadata
    const updatedCustomer = await billingPrisma.billing_customers.update({
      where: { id: customerId },
      data: {
        metadata: {
          ...currentMetadata,
          tax_exemptions: taxExemptions
        },
        updated_at: new Date()
      }
    });

    logger.info('Tax exemption created', {
      app_id: appId,
      customer_id: customerId,
      tax_type: taxType,
      certificate_number: certificateNumber
    });

    return newExemption;
  }

  /**
   * Record tax calculation (called during invoice generation)
   * @param {string} appId - Application identifier
   * @param {number} taxRateId - Tax rate used
   * @param {number} taxableAmountCents - Amount that was taxed
   * @param {number} taxAmountCents - Calculated tax amount
   * @param {Object} options - Optional parameters
   * @param {number} options.invoiceId - Invoice ID (if applicable)
   * @param {number} options.chargeId - Charge ID (if applicable)
   * @returns {Promise<Object>} Tax calculation record
   */
  async recordTaxCalculation(appId, taxRateId, taxableAmountCents, taxAmountCents, options = {}) {
    if (!appId || !taxRateId) {
      throw new ValidationError('appId and taxRateId are required');
    }

    if (typeof taxableAmountCents !== 'number' || typeof taxAmountCents !== 'number') {
      throw new ValidationError('taxableAmountCents and taxAmountCents must be numbers');
    }

    // Get tax rate to validate and get details
    const taxRate = await billingPrisma.billing_tax_rates.findFirst({
      where: {
        id: taxRateId,
        app_id: appId
      }
    });

    if (!taxRate) {
      throw new NotFoundError(`Tax rate ${taxRateId} not found for app ${appId}`);
    }

    // Create tax calculation record
    const taxCalculation = await billingPrisma.billing_tax_calculations.create({
      data: {
        app_id: appId,
        invoice_id: options.invoiceId || null,
        charge_id: options.chargeId || null,
        tax_rate_id: taxRateId,
        taxable_amount_cents: taxableAmountCents,
        tax_amount_cents: taxAmountCents,
        jurisdiction_code: taxRate.jurisdiction_code,
        tax_type: taxRate.tax_type,
        rate_applied: taxRate.rate
      }
    });

    logger.info('Tax calculation recorded', {
      app_id: appId,
      tax_calculation_id: taxCalculation.id,
      invoice_id: options.invoiceId,
      charge_id: options.chargeId,
      tax_amount_cents: taxAmountCents
    });

    return taxCalculation;
  }

  /**
   * Get tax calculations for an invoice
   * @param {string} appId - Application identifier
   * @param {number} invoiceId - Invoice ID
   * @returns {Promise<Array>} Array of tax calculations
   */
  async getTaxCalculationsForInvoice(appId, invoiceId) {
    if (!appId || !invoiceId) {
      throw new ValidationError('appId and invoiceId are required');
    }

    const taxCalculations = await billingPrisma.billing_tax_calculations.findMany({
      where: {
        app_id: appId,
        invoice_id: invoiceId
      },
      include: {
        tax_rate: true
      },
      orderBy: {
        created_at: 'asc'
      }
    });

    return taxCalculations;
  }

  /**
   * Get tax calculations for a charge
   * @param {string} appId - Application identifier
   * @param {number} chargeId - Charge ID
   * @returns {Promise<Array>} Array of tax calculations
   */
  async getTaxCalculationsForCharge(appId, chargeId) {
    if (!appId || !chargeId) {
      throw new ValidationError('appId and chargeId are required');
    }

    const taxCalculations = await billingPrisma.billing_tax_calculations.findMany({
      where: {
        app_id: appId,
        charge_id: chargeId
      },
      include: {
        tax_rate: true
      },
      orderBy: {
        created_at: 'asc'
      }
    });

    return taxCalculations;
  }
}

module.exports = TaxService;
