/**
 * Cross-Tenant Data Isolation Tests
 *
 * Verifies that one tenant (app_id) cannot access another tenant's data.
 * Pattern: create/mock a resource under app_id 'trashtech', attempt
 * access under app_id 'apping', and verify 404 NotFoundError.
 *
 * SOC 2 CC6.1 / PCI DSS Req 7: Logical access controls between tenants.
 */

const { NotFoundError } = require('../../backend/src/utils/errors');

// ─── Shared Prisma mock ─────────────────────────────────────────────────────

const mockPrisma = {
  billing_customers: {
    findFirst: jest.fn(),
    create: jest.fn(),
    update: jest.fn(),
  },
  billing_subscriptions: {
    findFirst: jest.fn(),
    findUnique: jest.fn(),
    update: jest.fn(),
  },
  billing_invoices: {
    findFirst: jest.fn(),
    create: jest.fn(),
  },
  billing_refunds: {
    findFirst: jest.fn(),
    findMany: jest.fn(),
    create: jest.fn(),
    update: jest.fn(),
    aggregate: jest.fn(),
  },
  billing_charges: {
    findFirst: jest.fn(),
    create: jest.fn(),
    update: jest.fn(),
  },
  billing_tax_rates: {
    findFirst: jest.fn(),
    findMany: jest.fn(),
  },
  billing_tax_exemptions: {
    findFirst: jest.fn(),
  },
  billing_metered_usage: {
    findMany: jest.fn(),
    create: jest.fn(),
    aggregate: jest.fn(),
  },
  billing_coupons: {
    findFirst: jest.fn(),
    findMany: jest.fn(),
  },
  billing_events: {
    create: jest.fn(),
  },
  billing_payment_methods: {
    findFirst: jest.fn(),
    findMany: jest.fn(),
  },
  $transaction: jest.fn(async (callback) => callback(mockPrisma)),
};

jest.mock('../../backend/src/prisma', () => ({
  billingPrisma: mockPrisma,
}));

jest.mock('../../backend/src/prisma.factory', () => ({
  getBillingPrisma: () => mockPrisma,
}));

jest.mock('@fireproof/infrastructure/utils/logger', () => ({
  info: jest.fn(),
  warn: jest.fn(),
  error: jest.fn(),
}));

// ─── Constants ───────────────────────────────────────────────────────────────

const APP_A = 'trashtech';  // Resource owner
const APP_B = 'apping';     // Cross-tenant attacker

// ─── CustomerService ─────────────────────────────────────────────────────────

describe('Cross-Tenant Isolation: CustomerService', () => {
  const CustomerService = require('../../backend/src/services/CustomerService');
  let service;

  beforeEach(() => {
    jest.clearAllMocks();
    service = new CustomerService(() => ({}));
  });

  it('getCustomerById: should reject access to another app\'s customer', async () => {
    // Customer 1 belongs to APP_A — query with APP_B returns null
    mockPrisma.billing_customers.findFirst.mockResolvedValue(null);

    await expect(service.getCustomerById(APP_B, 1)).rejects.toThrow(NotFoundError);

    // Verify the query was scoped to APP_B (not APP_A)
    expect(mockPrisma.billing_customers.findFirst).toHaveBeenCalledWith({
      where: { id: 1, app_id: APP_B },
    });
  });

  it('findCustomer: should reject access to another app\'s customer by external ID', async () => {
    mockPrisma.billing_customers.findFirst.mockResolvedValue(null);

    await expect(service.findCustomer(APP_B, '42')).rejects.toThrow(NotFoundError);

    expect(mockPrisma.billing_customers.findFirst).toHaveBeenCalledWith({
      where: { app_id: APP_B, external_customer_id: '42' },
    });
  });

  it('updateCustomer: should reject update to another app\'s customer', async () => {
    mockPrisma.billing_customers.findFirst.mockResolvedValue(null);

    await expect(
      service.updateCustomer(APP_B, 1, { name: 'Hijacked' })
    ).rejects.toThrow(NotFoundError);
  });
});

// ─── SubscriptionService ─────────────────────────────────────────────────────

describe('Cross-Tenant Isolation: SubscriptionService', () => {
  const SubscriptionService = require('../../backend/src/services/SubscriptionService');
  let service;

  beforeEach(() => {
    jest.clearAllMocks();
    service = new SubscriptionService(() => ({}));
  });

  it('getSubscriptionById: should reject when subscription belongs to different app', async () => {
    // Subscription exists but belongs to APP_A via customer
    mockPrisma.billing_subscriptions.findFirst.mockResolvedValue({
      id: 10,
      billing_customer_id: 1,
      billing_customers: { app_id: APP_A },
    });

    await expect(
      service.getSubscriptionById(APP_B, 10)
    ).rejects.toThrow(NotFoundError);
  });

  it('getSubscriptionById: should succeed when subscription belongs to same app', async () => {
    const sub = {
      id: 10,
      billing_customer_id: 1,
      billing_customers: { app_id: APP_A },
    };
    mockPrisma.billing_subscriptions.findFirst.mockResolvedValue(sub);

    const result = await service.getSubscriptionById(APP_A, 10);
    expect(result).toEqual(sub);
  });
});

// ─── InvoiceService ──────────────────────────────────────────────────────────

describe('Cross-Tenant Isolation: InvoiceService', () => {
  const InvoiceService = require('../../backend/src/services/InvoiceService');
  let service;

  beforeEach(() => {
    jest.clearAllMocks();
    service = new InvoiceService();
  });

  it('getInvoice: should reject access to another app\'s invoice', async () => {
    mockPrisma.billing_invoices.findFirst.mockResolvedValue(null);

    await expect(service.getInvoice(APP_B, 5)).rejects.toThrow(NotFoundError);

    expect(mockPrisma.billing_invoices.findFirst).toHaveBeenCalledWith(
      expect.objectContaining({
        where: { id: 5, app_id: APP_B },
      })
    );
  });

  it('updateInvoiceStatus: should reject status change on another app\'s invoice', async () => {
    mockPrisma.billing_invoices.findFirst.mockResolvedValue(null);

    await expect(
      service.updateInvoiceStatus(APP_B, 5, 'paid')
    ).rejects.toThrow(NotFoundError);
  });
});

// ─── RefundService ───────────────────────────────────────────────────────────

describe('Cross-Tenant Isolation: RefundService', () => {
  const RefundService = require('../../backend/src/services/RefundService');
  let service;

  beforeEach(() => {
    jest.clearAllMocks();
    service = new RefundService(() => ({}));
  });

  it('getRefund: should reject access to another app\'s refund', async () => {
    mockPrisma.billing_refunds.findFirst.mockResolvedValue(null);

    await expect(service.getRefund(APP_B, 7)).rejects.toThrow(NotFoundError);

    expect(mockPrisma.billing_refunds.findFirst).toHaveBeenCalledWith(
      expect.objectContaining({
        where: expect.objectContaining({ id: 7, app_id: APP_B }),
      })
    );
  });

  it('listRefunds: should scope query to requesting app', async () => {
    mockPrisma.billing_refunds.findMany.mockResolvedValue([]);

    const result = await service.listRefunds(APP_B, {});

    expect(result).toEqual([]);
    expect(mockPrisma.billing_refunds.findMany).toHaveBeenCalledWith(
      expect.objectContaining({
        where: expect.objectContaining({ app_id: APP_B }),
      })
    );
  });
});

// ─── ProrationService ────────────────────────────────────────────────────────

describe('Cross-Tenant Isolation: ProrationService', () => {
  const ProrationService = require('../../backend/src/services/ProrationService');
  const service = new ProrationService();

  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('calculateProration: should reject when subscription belongs to different app', async () => {
    mockPrisma.billing_subscriptions.findUnique.mockResolvedValue({
      id: 10,
      status: 'active',
      price_cents: 5000,
      current_period_start: new Date('2026-01-01'),
      current_period_end: new Date('2026-02-01'),
      billing_customers: { app_id: APP_A },
    });

    await expect(
      service.calculateProration({
        subscriptionId: 10,
        changeDate: new Date('2026-01-15'),
        newPriceCents: 8000,
        oldPriceCents: 5000,
        appId: APP_B,
      })
    ).rejects.toThrow(NotFoundError);
  });

  it('calculateProration: should succeed when subscription belongs to same app', async () => {
    mockPrisma.billing_subscriptions.findUnique.mockResolvedValue({
      id: 10,
      status: 'active',
      price_cents: 5000,
      current_period_start: new Date('2026-01-01'),
      current_period_end: new Date('2026-02-01'),
      billing_customers: { app_id: APP_A },
    });

    // Should not throw for correct app
    const result = await service.calculateProration({
      subscriptionId: 10,
      changeDate: new Date('2026-01-15'),
      newPriceCents: 8000,
      oldPriceCents: 5000,
      appId: APP_A,
    });

    expect(result).toBeDefined();
    expect(result.old_plan.credit_cents).toBeDefined();
    expect(result.new_plan.charge_cents).toBeDefined();
  });
});

// ─── TaxService ──────────────────────────────────────────────────────────────

describe('Cross-Tenant Isolation: TaxService', () => {
  const TaxService = require('../../backend/src/services/TaxService');
  const service = new TaxService();

  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('calculateTax: should reject when customer belongs to different app', async () => {
    mockPrisma.billing_customers.findFirst.mockResolvedValue(null);

    await expect(
      service.calculateTax(APP_B, 1, 10000)
    ).rejects.toThrow(NotFoundError);

    expect(mockPrisma.billing_customers.findFirst).toHaveBeenCalledWith({
      where: { id: 1, app_id: APP_B },
    });
  });

  it('getTaxRatesByJurisdiction: should scope query to requesting app', async () => {
    mockPrisma.billing_tax_rates.findMany.mockResolvedValue([]);

    const result = await service.getTaxRatesByJurisdiction(APP_B, 'US-CA');

    expect(result).toEqual([]);
    expect(mockPrisma.billing_tax_rates.findMany).toHaveBeenCalledWith(
      expect.objectContaining({
        where: expect.objectContaining({ app_id: APP_B }),
      })
    );
  });
});

// ─── UsageService ────────────────────────────────────────────────────────────

describe('Cross-Tenant Isolation: UsageService', () => {
  const UsageService = require('../../backend/src/services/UsageService');
  const service = new UsageService();

  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('recordUsage: should reject when customer belongs to different app', async () => {
    mockPrisma.billing_customers.findFirst.mockResolvedValue(null);

    await expect(
      service.recordUsage({
        appId: APP_B,
        customerId: 1,
        subscriptionId: 10,
        metricName: 'api_calls',
        quantity: 100,
        unitPriceCents: 10,
        periodStart: new Date('2026-01-01'),
        periodEnd: new Date('2026-02-01'),
      })
    ).rejects.toThrow(NotFoundError);
  });
});

// ─── DiscountService ─────────────────────────────────────────────────────────

describe('Cross-Tenant Isolation: DiscountService', () => {
  const DiscountService = require('../../backend/src/services/DiscountService');
  const service = new DiscountService();

  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('calculateDiscounts: should reject when customer belongs to different app', async () => {
    mockPrisma.billing_customers.findFirst.mockResolvedValue(null);

    await expect(
      service.calculateDiscounts(APP_B, 1, 10000, { couponCodes: ['SAVE10'] })
    ).rejects.toThrow(NotFoundError);
  });

  it('validateCoupon: should not find coupons from another app', async () => {
    // Coupon exists in APP_A but query for APP_B returns null
    mockPrisma.billing_coupons.findFirst.mockResolvedValue(null);

    const result = await service.validateCoupon(APP_B, 'SAVE10');

    expect(result.valid).toBe(false);
    expect(result.reason).toMatch(/not found/i);
    expect(mockPrisma.billing_coupons.findFirst).toHaveBeenCalledWith(
      expect.objectContaining({
        where: expect.objectContaining({ app_id: APP_B }),
      })
    );
  });
});

// ─── PaymentMethodService ────────────────────────────────────────────────────

describe('Cross-Tenant Isolation: PaymentMethodService', () => {
  const PaymentMethodService = require('../../backend/src/services/PaymentMethodService');
  let service;

  beforeEach(() => {
    jest.clearAllMocks();
    // PaymentMethodService constructor: (getTilledClientFn, customerService)
    const CustomerService = require('../../backend/src/services/CustomerService');
    const customerService = new CustomerService(() => ({}));
    service = new PaymentMethodService(() => ({}), customerService);
  });

  it('listPaymentMethods: should reject when customer belongs to different app', async () => {
    // CustomerService.getCustomerById will fail because findFirst returns null
    mockPrisma.billing_customers.findFirst.mockResolvedValue(null);

    await expect(
      service.listPaymentMethods(APP_B, 1)
    ).rejects.toThrow(NotFoundError);
  });
});

// ─── ChargeService ───────────────────────────────────────────────────────────

describe('Cross-Tenant Isolation: ChargeService', () => {
  const ChargeService = require('../../backend/src/services/ChargeService');
  let service;

  beforeEach(() => {
    jest.clearAllMocks();
    service = new ChargeService(() => ({}));
  });

  it('createOneTimeCharge: should reject when customer belongs to different app', async () => {
    mockPrisma.billing_customers.findFirst.mockResolvedValue(null);

    await expect(
      service.createOneTimeCharge(APP_B, {
        externalCustomerId: '42',
        paymentMethodId: 'pm_123',
        paymentMethodType: 'card',
        amountCents: 3500,
        reason: 'extra_pickup',
        referenceId: 'ref_cross_tenant_test',
      }, {
        idempotencyKey: 'idem_test',
        requestHash: 'hash_test',
      })
    ).rejects.toThrow(NotFoundError);

    // Verify query was scoped to APP_B
    expect(mockPrisma.billing_customers.findFirst).toHaveBeenCalledWith(
      expect.objectContaining({
        where: expect.objectContaining({ app_id: APP_B }),
      })
    );
  });
});
