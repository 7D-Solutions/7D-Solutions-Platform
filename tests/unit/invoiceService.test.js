const InvoiceService = require('../../backend/src/services/InvoiceService');
const { billingPrisma } = require('../../backend/src/prisma');
const { NotFoundError, ValidationError } = require('../../backend/src/utils/errors');

// Mock Prisma client
jest.mock('../../backend/src/prisma', () => {
  const prismaMock = {
    billing_invoices: {
      create: jest.fn(),
      findFirst: jest.fn(),
      findMany: jest.fn(),
      update: jest.fn(),
      count: jest.fn()
    },
    billing_invoice_line_items: {
      create: jest.fn(),
      aggregate: jest.fn()
    },
    billing_customers: {
      findFirst: jest.fn()
    },
    billing_subscriptions: {
      findFirst: jest.fn()
    },
    $transaction: jest.fn()
  };

  // Implement $transaction to call callback with the mock itself
  prismaMock.$transaction.mockImplementation((callback) => {
    return callback(prismaMock);
  });

  return { billingPrisma: prismaMock };
});

// Mock logger
jest.mock('@fireproof/infrastructure/utils/logger', () => ({
  info: jest.fn(),
  warn: jest.fn(),
  error: jest.fn()
}));

describe('InvoiceService', () => {
  let invoiceService;
  const mockGetTilledClient = jest.fn();

  beforeEach(() => {
    invoiceService = new InvoiceService(mockGetTilledClient);
    jest.clearAllMocks();
  });

  describe('createInvoice', () => {
    const validParams = {
      appId: 'testapp',
      customerId: 123,
      amountCents: 5000,
      subscriptionId: 456,
      status: 'draft',
      currency: 'usd',
      dueAt: new Date('2026-12-31'),
      metadata: { note: 'test' },
      billingPeriodStart: new Date('2026-01-01'),
      billingPeriodEnd: new Date('2026-01-31'),
      lineItemDetails: { items: [] },
      complianceCodes: { region: 'US' }
    };

    const mockCustomer = {
      id: 123,
      email: 'customer@example.com',
      name: 'Test Customer'
    };

    const mockSubscription = {
      id: 456,
      plan_name: 'Premium Plan',
      price_cents: 5000
    };

    const mockInvoice = {
      id: 789,
      app_id: 'testapp',
      tilled_invoice_id: 'in_testapp_12345',
      billing_customer_id: 123,
      subscription_id: 456,
      status: 'draft',
      amount_cents: 5000,
      currency: 'usd',
      due_at: new Date('2026-12-31'),
      metadata: { note: 'test' },
      billing_period_start: new Date('2026-01-01'),
      billing_period_end: new Date('2026-01-31'),
      line_item_details: { items: [] },
      compliance_codes: { region: 'US' },
      created_at: new Date(),
      updated_at: new Date()
    };

    beforeEach(() => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockSubscription);
      billingPrisma.billing_invoices.create.mockResolvedValue(mockInvoice);
    });

    it('should create invoice with all parameters', async () => {
      const result = await invoiceService.createInvoice(validParams);

      expect(billingPrisma.billing_customers.findFirst).toHaveBeenCalledWith({
        where: { id: 123, app_id: 'testapp' }
      });
      expect(billingPrisma.billing_subscriptions.findFirst).toHaveBeenCalledWith({
        where: { id: 456, app_id: 'testapp', billing_customer_id: 123 }
      });
      expect(billingPrisma.billing_invoices.create).toHaveBeenCalledWith({
        data: {
          app_id: 'testapp',
          tilled_invoice_id: expect.any(String),
          billing_customer_id: 123,
          subscription_id: 456,
          status: 'draft',
          amount_cents: 5000,
          currency: 'usd',
          due_at: new Date('2026-12-31'),
          metadata: { note: 'test' },
          billing_period_start: new Date('2026-01-01'),
          billing_period_end: new Date('2026-01-31'),
          line_item_details: { items: [] },
          compliance_codes: { region: 'US' }
        }
      });
      expect(result).toEqual({
        id: 789,
        app_id: 'testapp',
        tilled_invoice_id: 'in_testapp_12345',
        billing_customer_id: 123,
        subscription_id: 456,
        status: 'draft',
        amount_cents: 5000,
        currency: 'usd',
        due_at: new Date('2026-12-31'),
        metadata: { note: 'test' },
        billing_period_start: new Date('2026-01-01'),
        billing_period_end: new Date('2026-01-31'),
        line_item_details: { items: [] },
        compliance_codes: { region: 'US' },
        created_at: mockInvoice.created_at,
        updated_at: mockInvoice.updated_at
      });
    });

    it('should create invoice without optional fields', async () => {
      const params = {
        appId: 'testapp',
        customerId: 123,
        amountCents: 5000
      };
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_invoices.create.mockResolvedValue({
        ...mockInvoice,
        subscription_id: null,
        due_at: null,
        metadata: {},
        billing_period_start: null,
        billing_period_end: null,
        line_item_details: null,
        compliance_codes: null
      });

      const result = await invoiceService.createInvoice(params);

      expect(billingPrisma.billing_subscriptions.findFirst).not.toHaveBeenCalled();
      expect(billingPrisma.billing_invoices.create).toHaveBeenCalledWith({
        data: {
          app_id: 'testapp',
          tilled_invoice_id: expect.any(String),
          billing_customer_id: 123,
          subscription_id: null,
          status: 'draft',
          amount_cents: 5000,
          currency: 'usd',
          due_at: null,
          metadata: {},
          billing_period_start: null,
          billing_period_end: null,
          line_item_details: null,
          compliance_codes: null
        }
      });
      expect(result.subscription_id).toBeNull();
    });

    it('should throw ValidationError if required fields missing', async () => {
      await expect(invoiceService.createInvoice({}))
        .rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if amountCents negative', async () => {
      await expect(invoiceService.createInvoice({
        appId: 'testapp',
        customerId: 123,
        amountCents: -100
      })).rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if status invalid', async () => {
      await expect(invoiceService.createInvoice({
        appId: 'testapp',
        customerId: 123,
        amountCents: 5000,
        status: 'invalid'
      })).rejects.toThrow(ValidationError);
    });

    it('should throw NotFoundError if customer not found', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);
      await expect(invoiceService.createInvoice(validParams))
        .rejects.toThrow(NotFoundError);
    });

    it('should throw NotFoundError if subscription not found', async () => {
      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(null);
      await expect(invoiceService.createInvoice(validParams))
        .rejects.toThrow(NotFoundError);
    });
  });

  describe('getInvoice', () => {
    const mockInvoice = {
      id: 789,
      app_id: 'testapp',
      billing_customer_id: 123,
      subscription_id: 456,
      status: 'draft',
      amount_cents: 5000,
      customer: {
        email: 'customer@example.com',
        name: 'Test Customer',
        external_customer_id: 'ext123'
      },
      subscription: {
        plan_name: 'Premium Plan',
        plan_id: 'plan_123',
        status: 'active'
      }
    };

    beforeEach(() => {
      billingPrisma.billing_invoices.findFirst.mockResolvedValue(mockInvoice);
    });

    it('should get invoice without line items', async () => {
      const result = await invoiceService.getInvoice('testapp', 789, false);

      expect(billingPrisma.billing_invoices.findFirst).toHaveBeenCalledWith({
        where: { id: 789, app_id: 'testapp' },
        include: {
          customer: { select: { email: true, name: true, external_customer_id: true } },
          subscription: { select: { plan_name: true, plan_id: true, status: true } }
        }
      });
      expect(result).toEqual(mockInvoice);
    });

    it('should get invoice with line items', async () => {
      const mockInvoiceWithLineItems = {
        ...mockInvoice,
        billing_invoice_line_items: [
          { id: 1, description: 'Line item 1', amount_cents: 2500 }
        ]
      };
      billingPrisma.billing_invoices.findFirst.mockResolvedValue(mockInvoiceWithLineItems);

      const result = await invoiceService.getInvoice('testapp', 789, true);

      expect(billingPrisma.billing_invoices.findFirst).toHaveBeenCalledWith({
        where: { id: 789, app_id: 'testapp' },
        include: {
          customer: { select: { email: true, name: true, external_customer_id: true } },
          subscription: { select: { plan_name: true, plan_id: true, status: true } },
          billing_invoice_line_items: true
        }
      });
      expect(result.billing_invoice_line_items).toHaveLength(1);
    });

    it('should throw ValidationError if appId or invoiceId missing', async () => {
      await expect(invoiceService.getInvoice(null, 789))
        .rejects.toThrow(ValidationError);
      await expect(invoiceService.getInvoice('testapp', null))
        .rejects.toThrow(ValidationError);
    });

    it('should throw NotFoundError if invoice not found', async () => {
      billingPrisma.billing_invoices.findFirst.mockResolvedValue(null);
      await expect(invoiceService.getInvoice('testapp', 789))
        .rejects.toThrow(NotFoundError);
    });
  });

  describe('addInvoiceLineItem', () => {
    const validParams = {
      appId: 'testapp',
      invoiceId: 789,
      lineItemType: 'subscription',
      description: 'Premium Plan subscription',
      quantity: 1,
      unitPriceCents: 5000,
      metadata: { plan_id: 'plan_123' }
    };

    const mockInvoice = {
      id: 789,
      app_id: 'testapp'
    };

    const mockLineItem = {
      id: 999,
      app_id: 'testapp',
      invoice_id: 789,
      line_item_type: 'subscription',
      description: 'Premium Plan subscription',
      quantity: 1,
      unit_price_cents: 5000,
      amount_cents: 5000,
      metadata: { plan_id: 'plan_123' },
      created_at: new Date()
    };

    beforeEach(() => {
      billingPrisma.billing_invoices.findFirst.mockResolvedValue(mockInvoice);
      billingPrisma.billing_invoice_line_items.create.mockResolvedValue(mockLineItem);
      billingPrisma.billing_invoice_line_items.aggregate.mockResolvedValue({ _sum: { amount_cents: 5000 } });
    });

    it('should add line item with valid parameters', async () => {
      const result = await invoiceService.addInvoiceLineItem(validParams);

      expect(billingPrisma.billing_invoices.findFirst).toHaveBeenCalledWith({
        where: { id: 789, app_id: 'testapp' }
      });
      expect(billingPrisma.billing_invoice_line_items.create).toHaveBeenCalledWith({
        data: {
          app_id: 'testapp',
          invoice_id: 789,
          line_item_type: 'subscription',
          description: 'Premium Plan subscription',
          quantity: 1,
          unit_price_cents: 5000,
          amount_cents: 5000,
          metadata: { plan_id: 'plan_123' }
        }
      });
      expect(result).toEqual({
        id: 999,
        app_id: 'testapp',
        invoice_id: 789,
        line_item_type: 'subscription',
        description: 'Premium Plan subscription',
        quantity: 1,
        unit_price_cents: 5000,
        amount_cents: 5000,
        metadata: { plan_id: 'plan_123' },
        created_at: mockLineItem.created_at
      });
    });

    it('should calculate amount correctly for decimal quantity', async () => {
      const params = { ...validParams, quantity: 2.5, unitPriceCents: 100 };
      billingPrisma.billing_invoice_line_items.create.mockImplementation(async ({ data }) => ({
        ...mockLineItem,
        quantity: data.quantity,
        unit_price_cents: data.unit_price_cents,
        amount_cents: data.amount_cents
      }));
      billingPrisma.billing_invoice_line_items.aggregate.mockResolvedValue({ _sum: { amount_cents: 250 } });

      const result = await invoiceService.addInvoiceLineItem(params);

      expect(result.amount_cents).toBe(250); // 2.5 * 100 = 250
    });

    it('should throw ValidationError if required fields missing', async () => {
      await expect(invoiceService.addInvoiceLineItem({}))
        .rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if quantity negative', async () => {
      await expect(invoiceService.addInvoiceLineItem({
        ...validParams,
        quantity: -1
      })).rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if unitPriceCents negative', async () => {
      await expect(invoiceService.addInvoiceLineItem({
        ...validParams,
        unitPriceCents: -100
      })).rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if lineItemType invalid', async () => {
      await expect(invoiceService.addInvoiceLineItem({
        ...validParams,
        lineItemType: 'invalid'
      })).rejects.toThrow(ValidationError);
    });

    it('should throw NotFoundError if invoice not found', async () => {
      billingPrisma.billing_invoices.findFirst.mockResolvedValue(null);
      await expect(invoiceService.addInvoiceLineItem(validParams))
        .rejects.toThrow(NotFoundError);
    });

    it('should throw ValidationError if invoice status is finalized', async () => {
      billingPrisma.billing_invoices.findFirst.mockResolvedValue({
        id: 789,
        app_id: 'testapp',
        status: 'paid'
      });
      await expect(invoiceService.addInvoiceLineItem(validParams))
        .rejects.toThrow(ValidationError);
    });
  });

  describe('generateInvoiceFromSubscription', () => {
    const validParams = {
      appId: 'testapp',
      subscriptionId: 456,
      billingPeriodStart: new Date('2026-01-01'),
      billingPeriodEnd: new Date('2026-01-31'),
      includeUsage: true,
      includeTax: true,
      includeDiscounts: true
    };

    const mockSubscription = {
      id: 456,
      billing_customer_id: 123,
      plan_name: 'Premium Plan',
      plan_id: 'plan_123',
      price_cents: 5000,
      billing_customers: {
        id: 123,
        email: 'customer@example.com'
      }
    };

    const mockInvoice = {
      id: 789,
      app_id: 'testapp',
      billing_customer_id: 123,
      subscription_id: 456,
      status: 'draft',
      amount_cents: 5000,
      billing_period_start: new Date('2026-01-01'),
      billing_period_end: new Date('2026-01-31'),
      line_item_details: {
        generated_from: 'subscription',
        period: { start: new Date('2026-01-01'), end: new Date('2026-01-31') },
        components: { subscription: true, usage: true, discounts: true, tax: true }
      }
    };

    beforeEach(() => {
      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockSubscription);
      // Mock createInvoice and addInvoiceLineItem methods
      invoiceService.createInvoice = jest.fn().mockResolvedValue(mockInvoice);
      invoiceService.addInvoiceLineItem = jest.fn();
      invoiceService.getInvoice = jest.fn().mockResolvedValue({
        ...mockInvoice,
        billing_invoice_line_items: []
      });
    });

    it('should generate invoice from subscription', async () => {
      const result = await invoiceService.generateInvoiceFromSubscription(validParams);

      expect(billingPrisma.billing_subscriptions.findFirst).toHaveBeenCalledWith({
        where: { id: 456, app_id: 'testapp' },
        include: { billing_customers: true }
      });
      expect(invoiceService.createInvoice).toHaveBeenCalledWith({
        appId: 'testapp',
        customerId: 123,
        subscriptionId: 456,
        status: 'draft',
        amountCents: 5000,
        billingPeriodStart: new Date('2026-01-01'),
        billingPeriodEnd: new Date('2026-01-31'),
        lineItemDetails: {
          generated_from: 'subscription',
          period: { start: new Date('2026-01-01'), end: new Date('2026-01-31') },
          components: { subscription: true, usage: true, discounts: true, tax: true }
        }
      });
      expect(invoiceService.addInvoiceLineItem).toHaveBeenCalledWith({
        appId: 'testapp',
        invoiceId: 789,
        lineItemType: 'subscription',
        description: 'Subscription: Premium Plan',
        quantity: 1,
        unitPriceCents: 5000,
        metadata: {
          plan_id: 'plan_123',
          plan_name: 'Premium Plan',
          billing_period_start: new Date('2026-01-01'),
          billing_period_end: new Date('2026-01-31')
        }
      });
      expect(invoiceService.getInvoice).toHaveBeenCalledWith('testapp', 789, true);
      expect(result).toBeDefined();
    });

    it('should throw ValidationError if appId missing', async () => {
      const params = { ...validParams, appId: undefined };
      await expect(invoiceService.generateInvoiceFromSubscription(params))
        .rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if subscriptionId missing', async () => {
      const params = { ...validParams, subscriptionId: undefined };
      await expect(invoiceService.generateInvoiceFromSubscription(params))
        .rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if billingPeriodStart missing', async () => {
      const params = { ...validParams, billingPeriodStart: undefined };
      await expect(invoiceService.generateInvoiceFromSubscription(params))
        .rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if billingPeriodEnd missing', async () => {
      const params = { ...validParams, billingPeriodEnd: undefined };
      await expect(invoiceService.generateInvoiceFromSubscription(params))
        .rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if billingPeriodStart >= billingPeriodEnd', async () => {
      const params = {
        ...validParams,
        billingPeriodStart: new Date('2026-01-31'),
        billingPeriodEnd: new Date('2026-01-01')
      };
      await expect(invoiceService.generateInvoiceFromSubscription(params))
        .rejects.toThrow(ValidationError);
    });

    it('should throw NotFoundError if subscription not found', async () => {
      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(null);
      await expect(invoiceService.generateInvoiceFromSubscription(validParams))
        .rejects.toThrow(NotFoundError);
    });
  });

  describe('updateInvoiceStatus', () => {
    const mockInvoice = {
      id: 789,
      app_id: 'testapp',
      status: 'draft',
      paid_at: null
    };

    const mockUpdatedInvoice = {
      ...mockInvoice,
      status: 'paid',
      paid_at: new Date('2026-01-15'),
      updated_at: new Date()
    };

    beforeEach(() => {
      billingPrisma.billing_invoices.findFirst.mockResolvedValue(mockInvoice);
      billingPrisma.billing_invoices.update.mockResolvedValue(mockUpdatedInvoice);
    });

    it('should update invoice status to paid', async () => {
      const result = await invoiceService.updateInvoiceStatus('testapp', 789, 'paid', {
        paidAt: new Date('2026-01-15')
      });

      expect(billingPrisma.billing_invoices.findFirst).toHaveBeenCalledWith({
        where: { id: 789, app_id: 'testapp' }
      });
      expect(billingPrisma.billing_invoices.update).toHaveBeenCalledWith({
        where: { id: 789 },
        data: {
          status: 'paid',
          updated_at: expect.any(Date),
          paid_at: new Date('2026-01-15')
        }
      });
      expect(result.status).toBe('paid');
    });

    it('should set paid_at to current date if not provided', async () => {
      const result = await invoiceService.updateInvoiceStatus('testapp', 789, 'paid', {});

      expect(billingPrisma.billing_invoices.update).toHaveBeenCalledWith({
        where: { id: 789 },
        data: {
          status: 'paid',
          updated_at: expect.any(Date),
          paid_at: expect.any(Date)
        }
      });
      expect(result.paid_at).toBeDefined();
    });

    it('should update invoice status without paid_at for non-paid status', async () => {
      await invoiceService.updateInvoiceStatus('testapp', 789, 'void', {});

      expect(billingPrisma.billing_invoices.update).toHaveBeenCalledWith({
        where: { id: 789 },
        data: {
          status: 'void',
          updated_at: expect.any(Date)
        }
      });
    });

    it('should throw ValidationError for invalid status', async () => {
      await expect(invoiceService.updateInvoiceStatus('testapp', 789, 'invalid', {}))
        .rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if appId missing', async () => {
      await expect(invoiceService.updateInvoiceStatus(undefined, 789, 'paid', {}))
        .rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if invoiceId missing', async () => {
      await expect(invoiceService.updateInvoiceStatus('testapp', undefined, 'paid', {}))
        .rejects.toThrow(ValidationError);
    });

    it('should throw ValidationError if status missing', async () => {
      await expect(invoiceService.updateInvoiceStatus('testapp', 789, undefined, {}))
        .rejects.toThrow(ValidationError);
    });

    it('should throw NotFoundError if invoice not found', async () => {
      billingPrisma.billing_invoices.findFirst.mockResolvedValue(null);
      await expect(invoiceService.updateInvoiceStatus('testapp', 789, 'paid', {}))
        .rejects.toThrow(NotFoundError);
    });

    it('should reject invalid status transitions', async () => {
      // void is terminal — cannot transition to anything
      const voidInvoice = { ...mockInvoice, status: 'void' };
      billingPrisma.billing_invoices.findFirst.mockResolvedValue(voidInvoice);
      await expect(invoiceService.updateInvoiceStatus('testapp', 789, 'paid', {}))
        .rejects.toThrow(ValidationError);
      await expect(invoiceService.updateInvoiceStatus('testapp', 789, 'draft', {}))
        .rejects.toThrow(ValidationError);

      // paid can only go to void
      const paidInvoice = { ...mockInvoice, status: 'paid', paid_at: new Date() };
      billingPrisma.billing_invoices.findFirst.mockResolvedValue(paidInvoice);
      await expect(invoiceService.updateInvoiceStatus('testapp', 789, 'draft', {}))
        .rejects.toThrow(ValidationError);
      await expect(invoiceService.updateInvoiceStatus('testapp', 789, 'open', {}))
        .rejects.toThrow(ValidationError);
    });

    it('should allow valid status transitions', async () => {
      billingPrisma.billing_invoices.update.mockResolvedValue({ ...mockInvoice, status: 'open' });

      // draft → open (valid)
      billingPrisma.billing_invoices.findFirst.mockResolvedValue({ ...mockInvoice, status: 'draft' });
      await invoiceService.updateInvoiceStatus('testapp', 789, 'open', {});
      expect(billingPrisma.billing_invoices.update).toHaveBeenCalled();

      // open → paid (valid)
      billingPrisma.billing_invoices.findFirst.mockResolvedValue({ ...mockInvoice, status: 'open' });
      billingPrisma.billing_invoices.update.mockResolvedValue({ ...mockInvoice, status: 'paid', paid_at: new Date() });
      await invoiceService.updateInvoiceStatus('testapp', 789, 'paid', {});

      // paid → void (valid)
      billingPrisma.billing_invoices.findFirst.mockResolvedValue({ ...mockInvoice, status: 'paid', paid_at: new Date() });
      billingPrisma.billing_invoices.update.mockResolvedValue({ ...mockInvoice, status: 'void' });
      await invoiceService.updateInvoiceStatus('testapp', 789, 'void', {});

      // uncollectible → paid (valid)
      billingPrisma.billing_invoices.findFirst.mockResolvedValue({ ...mockInvoice, status: 'uncollectible' });
      billingPrisma.billing_invoices.update.mockResolvedValue({ ...mockInvoice, status: 'paid', paid_at: new Date() });
      await invoiceService.updateInvoiceStatus('testapp', 789, 'paid', {});
    });
  });

  describe('listInvoices', () => {
    const mockInvoices = [
      {
        id: 789,
        app_id: 'testapp',
        billing_customer_id: 123,
        amount_cents: 5000,
        status: 'paid',
        customer: { email: 'customer@example.com', name: 'Test Customer' },
        subscription: { plan_name: 'Premium Plan' }
      },
      {
        id: 790,
        app_id: 'testapp',
        billing_customer_id: 124,
        amount_cents: 3000,
        status: 'draft',
        customer: { email: 'customer2@example.com', name: 'Test Customer 2' },
        subscription: { plan_name: 'Basic Plan' }
      }
    ];

    beforeEach(() => {
      billingPrisma.billing_invoices.count.mockResolvedValue(2);
      billingPrisma.billing_invoices.findMany.mockResolvedValue(mockInvoices);
    });

    it('should list invoices with default pagination', async () => {
      const result = await invoiceService.listInvoices({ appId: 'testapp' });

      expect(billingPrisma.billing_invoices.count).toHaveBeenCalledWith({
        where: { app_id: 'testapp' }
      });
      expect(billingPrisma.billing_invoices.findMany).toHaveBeenCalledWith({
        where: { app_id: 'testapp' },
        include: {
          customer: { select: { email: true, name: true } },
          subscription: { select: { plan_name: true } }
        },
        orderBy: { created_at: 'desc' },
        take: 50,
        skip: 0
      });
      expect(result.invoices).toHaveLength(2);
      expect(result.pagination.total).toBe(2);
      expect(result.pagination.limit).toBe(50);
      expect(result.pagination.offset).toBe(0);
      expect(result.pagination.hasMore).toBe(false);
    });

    it('should apply customer filter', async () => {
      await invoiceService.listInvoices({
        appId: 'testapp',
        customerId: 123
      });

      expect(billingPrisma.billing_invoices.count).toHaveBeenCalledWith({
        where: { app_id: 'testapp', billing_customer_id: 123 }
      });
    });

    it('should apply subscription filter', async () => {
      await invoiceService.listInvoices({
        appId: 'testapp',
        subscriptionId: 456
      });

      expect(billingPrisma.billing_invoices.count).toHaveBeenCalledWith({
        where: { app_id: 'testapp', subscription_id: 456 }
      });
    });

    it('should apply status filter', async () => {
      await invoiceService.listInvoices({
        appId: 'testapp',
        status: 'paid'
      });

      expect(billingPrisma.billing_invoices.count).toHaveBeenCalledWith({
        where: { app_id: 'testapp', status: 'paid' }
      });
    });

    it('should apply date range filter', async () => {
      const startDate = new Date('2026-01-01');
      const endDate = new Date('2026-01-31');
      await invoiceService.listInvoices({
        appId: 'testapp',
        startDate,
        endDate
      });

      expect(billingPrisma.billing_invoices.count).toHaveBeenCalledWith({
        where: {
          app_id: 'testapp',
          created_at: { gte: startDate, lte: endDate }
        }
      });
    });

    it('should apply pagination limits', async () => {
      await invoiceService.listInvoices({
        appId: 'testapp',
        limit: 10,
        offset: 20
      });

      expect(billingPrisma.billing_invoices.findMany).toHaveBeenCalledWith({
        where: { app_id: 'testapp' },
        include: expect.anything(),
        orderBy: { created_at: 'desc' },
        take: 10,
        skip: 20
      });
    });

    it('should throw ValidationError if appId missing', async () => {
      await expect(invoiceService.listInvoices({}))
        .rejects.toThrow(ValidationError);
    });
  });
});