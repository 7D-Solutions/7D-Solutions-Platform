const TaxService = require('../../backend/src/services/TaxService');
const { billingPrisma } = require('../../backend/src/prisma');
const { NotFoundError, ValidationError } = require('../../backend/src/utils/errors');

// Mock Prisma client
jest.mock('../../backend/src/prisma', () => ({
  billingPrisma: {
    billing_customers: {
      findFirst: jest.fn()
    },
    billing_tax_rates: {
      findMany: jest.fn(),
      findFirst: jest.fn(),
      create: jest.fn()
    },
    billing_tax_calculations: {
      create: jest.fn(),
      findMany: jest.fn()
    }
  }
}));

// Mock logger
jest.mock('@fireproof/infrastructure/utils/logger', () => ({
  info: jest.fn(),
  warn: jest.fn(),
  error: jest.fn()
}));

describe('TaxService', () => {
  let taxService;
  const appId = 'trashtech';
  const customerId = 1;

  beforeEach(() => {
    taxService = new TaxService();
    jest.clearAllMocks();
  });

  describe('calculateTax', () => {
    const subtotalCents = 10000; // $100.00
    const mockCustomer = {
      id: customerId,
      app_id: appId,
      email: 'test@example.com',
      metadata: { jurisdiction_code: 'CA' }
    };

    const mockTaxRate = {
      id: 1,
      app_id: appId,
      jurisdiction_code: 'CA',
      tax_type: 'sales_tax',
      rate: 0.0825, // 8.25%
      effective_date: new Date('2024-01-01'),
      expiration_date: null,
      description: 'California Sales Tax'
    };

    it('should calculate tax for a valid customer and amount', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_tax_rates.findMany.mockResolvedValue([mockTaxRate]);

      const result = await taxService.calculateTax(appId, customerId, subtotalCents);

      expect(result.taxAmountCents).toBe(825); // $8.25
      expect(result.taxRate).toBeCloseTo(0.0825);
      expect(result.jurisdictionCode).toBe('CA');
      expect(result.taxType).toBe('sales_tax');
      expect(result.breakdown).toHaveLength(1);
      expect(result.breakdown[0].taxRateId).toBe(1);
    });

    it('should handle multiple tax rates', async () => {
      const mockTaxRates = [
        { ...mockTaxRate, id: 1, tax_type: 'sales_tax', rate: 0.0725 },
        { ...mockTaxRate, id: 2, tax_type: 'waste_fee', rate: 0.0100 }
      ];

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_tax_rates.findMany.mockResolvedValue(mockTaxRates);

      const result = await taxService.calculateTax(appId, customerId, subtotalCents);

      expect(result.taxAmountCents).toBe(825); // $7.25 + $1.00 = $8.25
      expect(result.breakdown).toHaveLength(2);
      expect(result.taxType).toBe('sales_tax, waste_fee');
    });

    it('should return zero tax for tax-exempt customers', async () => {
      const result = await taxService.calculateTax(appId, customerId, subtotalCents, {
        taxExempt: true
      });

      expect(result.taxAmountCents).toBe(0);
      expect(result.taxRate).toBe(0);
      expect(result.jurisdictionCode).toBe('EXEMPT');
      expect(result.taxType).toBe('exempt');
      expect(result.breakdown).toHaveLength(0);
    });

    it('should use jurisdiction override if provided', async () => {
      const mockNYCustomer = { ...mockCustomer, metadata: {} };
      const mockNYTaxRate = { ...mockTaxRate, jurisdiction_code: 'NY-NYC', rate: 0.08875 };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockNYCustomer);
      billingPrisma.billing_tax_rates.findMany.mockResolvedValue([mockNYTaxRate]);

      const result = await taxService.calculateTax(appId, customerId, subtotalCents, {
        jurisdictionCode: 'NY-NYC'
      });

      expect(result.jurisdictionCode).toBe('NY-NYC');
      expect(billingPrisma.billing_tax_rates.findMany).toHaveBeenCalledWith(
        expect.objectContaining({
          where: expect.objectContaining({
            jurisdiction_code: 'NY-NYC'
          })
        })
      );
    });

    it('should return zero tax when no tax rates found', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_tax_rates.findMany.mockResolvedValue([]);

      const result = await taxService.calculateTax(appId, customerId, subtotalCents);

      expect(result.taxAmountCents).toBe(0);
      expect(result.taxRate).toBe(0);
      expect(result.taxType).toBe('none');
      expect(result.breakdown).toHaveLength(0);
    });

    it('should throw ValidationError for missing required fields', async () => {
      await expect(
        taxService.calculateTax(null, customerId, subtotalCents)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.calculateTax(appId, null, subtotalCents)
      ).rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError for invalid amount', async () => {
      await expect(
        taxService.calculateTax(appId, customerId, -100)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.calculateTax(appId, customerId, 'invalid')
      ).rejects.toThrow(ValidationError);
    });

    it('should throw NotFoundError for non-existent customer', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(
        taxService.calculateTax(appId, customerId, subtotalCents)
      ).rejects.toThrow(NotFoundError);
    });

    it('should handle zero amount', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_tax_rates.findMany.mockResolvedValue([mockTaxRate]);

      const result = await taxService.calculateTax(appId, customerId, 0);

      expect(result.taxAmountCents).toBe(0);
      expect(result.breakdown[0].taxAmountCents).toBe(0);
    });

    it('should extract jurisdiction from customer state metadata', async () => {
      const customerWithState = {
        ...mockCustomer,
        metadata: { state: 'TX' }
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(customerWithState);
      billingPrisma.billing_tax_rates.findMany.mockResolvedValue([]);

      await taxService.calculateTax(appId, customerId, subtotalCents);

      expect(billingPrisma.billing_tax_rates.findMany).toHaveBeenCalledWith(
        expect.objectContaining({
          where: expect.objectContaining({
            jurisdiction_code: 'TX'
          })
        })
      );
    });
  });

  describe('getTaxRatesByJurisdiction', () => {
    const mockTaxRates = [
      {
        id: 1,
        app_id: appId,
        jurisdiction_code: 'CA',
        tax_type: 'sales_tax',
        rate: 0.0725,
        effective_date: new Date('2024-01-01'),
        expiration_date: null,
        description: 'California State Sales Tax',
        metadata: null
      }
    ];

    it('should return active tax rates for jurisdiction', async () => {
      billingPrisma.billing_tax_rates.findMany.mockResolvedValue(mockTaxRates);

      const result = await taxService.getTaxRatesByJurisdiction(appId, 'CA');

      expect(result).toHaveLength(1);
      expect(result[0].jurisdictionCode).toBe('CA');
      expect(result[0].rate).toBe(0.0725);
      expect(result[0].taxType).toBe('sales_tax');
    });

    it('should filter by effective and expiration dates', async () => {
      billingPrisma.billing_tax_rates.findMany.mockResolvedValue(mockTaxRates);

      await taxService.getTaxRatesByJurisdiction(appId, 'CA');

      const whereClause = billingPrisma.billing_tax_rates.findMany.mock.calls[0][0].where;
      expect(whereClause.effective_date).toBeDefined();
      expect(whereClause.OR).toBeDefined();
    });

    it('should throw ValidationError for missing parameters', async () => {
      await expect(
        taxService.getTaxRatesByJurisdiction(null, 'CA')
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.getTaxRatesByJurisdiction(appId, null)
      ).rejects.toThrow(ValidationError);
    });

    it('should return empty array when no rates found', async () => {
      billingPrisma.billing_tax_rates.findMany.mockResolvedValue([]);

      const result = await taxService.getTaxRatesByJurisdiction(appId, 'UNKNOWN');

      expect(result).toEqual([]);
    });
  });

  describe('createTaxRate', () => {
    const jurisdictionCode = 'CA';
    const taxType = 'sales_tax';
    const rate = 0.0825;

    const mockCreatedTaxRate = {
      id: 1,
      app_id: appId,
      jurisdiction_code: jurisdictionCode,
      tax_type: taxType,
      rate: rate,
      effective_date: new Date(),
      expiration_date: null,
      description: null,
      metadata: null
    };

    it('should create a new tax rate', async () => {
      billingPrisma.billing_tax_rates.create.mockResolvedValue(mockCreatedTaxRate);

      const result = await taxService.createTaxRate(appId, jurisdictionCode, taxType, rate);

      expect(result.id).toBe(1);
      expect(result.jurisdiction_code).toBe(jurisdictionCode);
      expect(result.tax_type).toBe(taxType);
      expect(result.rate).toBe(rate);
    });

    it('should accept optional parameters', async () => {
      const options = {
        effectiveDate: new Date('2025-01-01'),
        expirationDate: new Date('2025-12-31'),
        description: 'Temporary tax rate',
        metadata: { source: 'manual' }
      };

      billingPrisma.billing_tax_rates.create.mockResolvedValue(mockCreatedTaxRate);

      await taxService.createTaxRate(appId, jurisdictionCode, taxType, rate, options);

      const createCall = billingPrisma.billing_tax_rates.create.mock.calls[0][0];
      expect(createCall.data.effective_date).toEqual(options.effectiveDate);
      expect(createCall.data.expiration_date).toEqual(options.expirationDate);
      expect(createCall.data.description).toBe(options.description);
      expect(createCall.data.metadata).toEqual(options.metadata);
    });

    it('should throw ValidationError for missing required fields', async () => {
      await expect(
        taxService.createTaxRate(null, jurisdictionCode, taxType, rate)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.createTaxRate(appId, null, taxType, rate)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.createTaxRate(appId, jurisdictionCode, null, rate)
      ).rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError for invalid rate', async () => {
      await expect(
        taxService.createTaxRate(appId, jurisdictionCode, taxType, -0.1)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.createTaxRate(appId, jurisdictionCode, taxType, 1.5)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.createTaxRate(appId, jurisdictionCode, taxType, 'invalid')
      ).rejects.toThrow(ValidationError);
    });
  });

  describe('createTaxExemption', () => {
    const taxType = 'sales_tax';
    const certificateNumber = 'EXEMPT-12345';

    const mockCustomer = {
      id: customerId,
      app_id: appId,
      email: 'test@example.com',
      metadata: {}
    };

    it('should create tax exemption for customer', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_customers.update = jest.fn().mockResolvedValue({
        ...mockCustomer,
        metadata: {
          tax_exemptions: [{ tax_type: taxType, certificate_number: certificateNumber, status: 'active' }]
        }
      });

      const result = await taxService.createTaxExemption(appId, customerId, taxType, certificateNumber);

      expect(result.tax_type).toBe(taxType);
      expect(result.certificate_number).toBe(certificateNumber);
      expect(result.status).toBe('active');
    });

    it('should preserve existing metadata', async () => {
      const customerWithMetadata = {
        ...mockCustomer,
        metadata: { other_field: 'value' }
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(customerWithMetadata);
      billingPrisma.billing_customers.update = jest.fn().mockResolvedValue(customerWithMetadata);

      await taxService.createTaxExemption(appId, customerId, taxType, certificateNumber);

      const updateCall = billingPrisma.billing_customers.update.mock.calls[0][0];
      expect(updateCall.data.metadata.other_field).toBe('value');
      expect(updateCall.data.metadata.tax_exemptions).toBeDefined();
    });

    it('should throw ValidationError for duplicate exemption', async () => {
      const customerWithExemption = {
        ...mockCustomer,
        metadata: {
          tax_exemptions: [
            { tax_type: taxType, certificate_number: certificateNumber, status: 'active' }
          ]
        }
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(customerWithExemption);

      await expect(
        taxService.createTaxExemption(appId, customerId, taxType, certificateNumber)
      ).rejects.toThrow(ValidationError);
    });

    it('should throw NotFoundError for non-existent customer', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(
        taxService.createTaxExemption(appId, customerId, taxType, certificateNumber)
      ).rejects.toThrow(NotFoundError);
    });

    it('should throw ValidationError for missing parameters', async () => {
      await expect(
        taxService.createTaxExemption(null, customerId, taxType, certificateNumber)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.createTaxExemption(appId, null, taxType, certificateNumber)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.createTaxExemption(appId, customerId, null, certificateNumber)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.createTaxExemption(appId, customerId, taxType, null)
      ).rejects.toThrow(ValidationError);
    });
  });

  describe('recordTaxCalculation', () => {
    const taxRateId = 1;
    const taxableAmountCents = 10000;
    const taxAmountCents = 825;

    const mockTaxRate = {
      id: taxRateId,
      app_id: appId,
      jurisdiction_code: 'CA',
      tax_type: 'sales_tax',
      rate: 0.0825
    };

    const mockTaxCalculation = {
      id: 1,
      app_id: appId,
      invoice_id: null,
      charge_id: null,
      tax_rate_id: taxRateId,
      taxable_amount: 100.00,
      tax_amount: 8.25,
      jurisdiction_code: 'CA',
      tax_type: 'sales_tax',
      rate_applied: 0.0825
    };

    it('should record tax calculation', async () => {
      billingPrisma.billing_tax_rates.findFirst.mockResolvedValue(mockTaxRate);
      billingPrisma.billing_tax_calculations.create.mockResolvedValue(mockTaxCalculation);

      const result = await taxService.recordTaxCalculation(
        appId,
        taxRateId,
        taxableAmountCents,
        taxAmountCents
      );

      expect(result.id).toBe(1);
      expect(result.tax_rate_id).toBe(taxRateId);
      expect(result.tax_amount).toBe(8.25);
    });

    it('should link to invoice when provided', async () => {
      const invoiceId = 123;

      billingPrisma.billing_tax_rates.findFirst.mockResolvedValue(mockTaxRate);
      billingPrisma.billing_tax_calculations.create.mockResolvedValue(mockTaxCalculation);

      await taxService.recordTaxCalculation(
        appId,
        taxRateId,
        taxableAmountCents,
        taxAmountCents,
        { invoiceId }
      );

      const createCall = billingPrisma.billing_tax_calculations.create.mock.calls[0][0];
      expect(createCall.data.invoice_id).toBe(invoiceId);
    });

    it('should link to charge when provided', async () => {
      const chargeId = 456;

      billingPrisma.billing_tax_rates.findFirst.mockResolvedValue(mockTaxRate);
      billingPrisma.billing_tax_calculations.create.mockResolvedValue(mockTaxCalculation);

      await taxService.recordTaxCalculation(
        appId,
        taxRateId,
        taxableAmountCents,
        taxAmountCents,
        { chargeId }
      );

      const createCall = billingPrisma.billing_tax_calculations.create.mock.calls[0][0];
      expect(createCall.data.charge_id).toBe(chargeId);
    });

    it('should throw NotFoundError for non-existent tax rate', async () => {
      billingPrisma.billing_tax_rates.findFirst.mockResolvedValue(null);

      await expect(
        taxService.recordTaxCalculation(appId, taxRateId, taxableAmountCents, taxAmountCents)
      ).rejects.toThrow(NotFoundError);
    });

    it('should throw ValidationError for missing parameters', async () => {
      await expect(
        taxService.recordTaxCalculation(null, taxRateId, taxableAmountCents, taxAmountCents)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.recordTaxCalculation(appId, null, taxableAmountCents, taxAmountCents)
      ).rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError for invalid amounts', async () => {
      billingPrisma.billing_tax_rates.findFirst.mockResolvedValue(mockTaxRate);

      await expect(
        taxService.recordTaxCalculation(appId, taxRateId, 'invalid', taxAmountCents)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.recordTaxCalculation(appId, taxRateId, taxableAmountCents, 'invalid')
      ).rejects.toThrow(ValidationError);
    });
  });

  describe('getTaxCalculationsForInvoice', () => {
    const invoiceId = 123;

    const mockTaxCalculations = [
      {
        id: 1,
        app_id: appId,
        invoice_id: invoiceId,
        charge_id: null,
        tax_rate_id: 1,
        taxable_amount: 100.00,
        tax_amount: 8.25,
        tax_rate: {
          id: 1,
          jurisdiction_code: 'CA',
          tax_type: 'sales_tax'
        }
      }
    ];

    it('should return tax calculations for invoice', async () => {
      billingPrisma.billing_tax_calculations.findMany.mockResolvedValue(mockTaxCalculations);

      const result = await taxService.getTaxCalculationsForInvoice(appId, invoiceId);

      expect(result).toHaveLength(1);
      expect(result[0].invoice_id).toBe(invoiceId);
      expect(result[0].tax_rate).toBeDefined();
    });

    it('should include tax rate details', async () => {
      billingPrisma.billing_tax_calculations.findMany.mockResolvedValue(mockTaxCalculations);

      await taxService.getTaxCalculationsForInvoice(appId, invoiceId);

      const findManyCall = billingPrisma.billing_tax_calculations.findMany.mock.calls[0][0];
      expect(findManyCall.include.tax_rate).toBe(true);
    });

    it('should throw ValidationError for missing parameters', async () => {
      await expect(
        taxService.getTaxCalculationsForInvoice(null, invoiceId)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.getTaxCalculationsForInvoice(appId, null)
      ).rejects.toThrow(ValidationError);
    });

    it('should return empty array when no calculations found', async () => {
      billingPrisma.billing_tax_calculations.findMany.mockResolvedValue([]);

      const result = await taxService.getTaxCalculationsForInvoice(appId, 999);

      expect(result).toEqual([]);
    });
  });

  describe('getTaxCalculationsForCharge', () => {
    const chargeId = 456;

    const mockTaxCalculations = [
      {
        id: 1,
        app_id: appId,
        invoice_id: null,
        charge_id: chargeId,
        tax_rate_id: 1,
        taxable_amount: 50.00,
        tax_amount: 4.13,
        tax_rate: {
          id: 1,
          jurisdiction_code: 'CA',
          tax_type: 'sales_tax'
        }
      }
    ];

    it('should return tax calculations for charge', async () => {
      billingPrisma.billing_tax_calculations.findMany.mockResolvedValue(mockTaxCalculations);

      const result = await taxService.getTaxCalculationsForCharge(appId, chargeId);

      expect(result).toHaveLength(1);
      expect(result[0].charge_id).toBe(chargeId);
      expect(result[0].tax_rate).toBeDefined();
    });

    it('should throw ValidationError for missing parameters', async () => {
      await expect(
        taxService.getTaxCalculationsForCharge(null, chargeId)
      ).rejects.toThrow(ValidationError);

      await expect(
        taxService.getTaxCalculationsForCharge(appId, null)
      ).rejects.toThrow(ValidationError);
    });

    it('should return empty array when no calculations found', async () => {
      billingPrisma.billing_tax_calculations.findMany.mockResolvedValue([]);

      const result = await taxService.getTaxCalculationsForCharge(appId, 999);

      expect(result).toEqual([]);
    });
  });
});
