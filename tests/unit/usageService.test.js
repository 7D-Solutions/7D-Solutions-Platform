const UsageService = require('../../backend/src/services/UsageService');
const { billingPrisma } = require('../../backend/src/prisma');
const { NotFoundError, ValidationError } = require('../../backend/src/utils/errors');

// Mock Prisma client
jest.mock('../../backend/src/prisma', () => ({
  billingPrisma: {
    billing_metered_usage: {
      create: jest.fn(),
      findMany: jest.fn(),
      updateMany: jest.fn()
    },
    billing_customers: {
      findFirst: jest.fn()
    },
    billing_subscriptions: {
      findFirst: jest.fn()
    }
  }
}));

// Mock logger
jest.mock('@fireproof/infrastructure/utils/logger', () => ({
  info: jest.fn(),
  warn: jest.fn(),
  error: jest.fn()
}));

describe('UsageService', () => {
  let usageService;
  const mockGetTilledClient = jest.fn();

  beforeEach(() => {
    usageService = new UsageService(mockGetTilledClient);
    jest.clearAllMocks();
  });

  describe('recordUsage', () => {
    const validParams = {
      appId: 'trashtech',
      customerId: 123,
      metricName: 'container_pickups',
      quantity: 10.5,
      unitPriceCents: 100, // $1.00 per pickup
      periodStart: new Date('2026-01-01T00:00:00Z'),
      periodEnd: new Date('2026-01-31T23:59:59Z')
    };

    const mockCustomer = {
      id: 123,
      app_id: 'trashtech',
      email: 'test@example.com',
      name: 'Test Customer'
    };

    beforeEach(() => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_metered_usage.create.mockImplementation(async ({ data }) => ({
        id: 1,
        app_id: data.app_id,
        customer_id: data.customer_id,
        subscription_id: data.subscription_id,
        metric_name: data.metric_name,
        quantity: data.quantity,
        unit_price_cents: data.unit_price_cents,
        period_start: data.period_start,
        period_end: data.period_end,
        recorded_at: data.recorded_at,
        billed_at: data.billed_at
      }));
    });

    it('should record usage with valid parameters', async () => {
      const result = await usageService.recordUsage(validParams);

      expect(billingPrisma.billing_customers.findFirst).toHaveBeenCalledWith({
        where: {
          id: validParams.customerId,
          app_id: validParams.appId
        }
      });

      expect(billingPrisma.billing_metered_usage.create).toHaveBeenCalledWith({
        data: {
          app_id: validParams.appId,
          customer_id: validParams.customerId,
          subscription_id: null,
          metric_name: validParams.metricName,
          quantity: validParams.quantity,
          unit_price_cents: validParams.unitPriceCents,
          period_start: validParams.periodStart,
          period_end: validParams.periodEnd,
          recorded_at: expect.any(Date),
          billed_at: null
        }
      });

      expect(result).toMatchObject({
        id: 1,
        app_id: validParams.appId,
        customer_id: validParams.customerId,
        metric_name: validParams.metricName,
        quantity: validParams.quantity,
        unit_price_cents: validParams.unitPriceCents,
        period_start: validParams.periodStart,
        period_end: validParams.periodEnd,
        billed_at: null
      });
    });

    it('should record usage with subscription ID', async () => {
      const paramsWithSubscription = {
        ...validParams,
        subscriptionId: 456
      };

      const mockSubscription = {
        id: 456,
        app_id: 'trashtech',
        billing_customer_id: 123,
        plan_name: 'Pro Plan'
      };

      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(mockSubscription);

      const result = await usageService.recordUsage(paramsWithSubscription);

      expect(billingPrisma.billing_subscriptions.findFirst).toHaveBeenCalledWith({
        where: {
          id: paramsWithSubscription.subscriptionId,
          app_id: paramsWithSubscription.appId,
          billing_customer_id: paramsWithSubscription.customerId
        }
      });

      expect(billingPrisma.billing_metered_usage.create).toHaveBeenCalledWith({
        data: {
          app_id: validParams.appId,
          customer_id: validParams.customerId,
          subscription_id: paramsWithSubscription.subscriptionId,
          metric_name: validParams.metricName,
          quantity: validParams.quantity,
          unit_price_cents: validParams.unitPriceCents,
          period_start: validParams.periodStart,
          period_end: validParams.periodEnd,
          recorded_at: expect.any(Date),
          billed_at: null
        }
      });

      expect(result.subscription_id).toBe(paramsWithSubscription.subscriptionId);
    });

    it('should throw NotFoundError when customer not found', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(usageService.recordUsage(validParams))
        .rejects
        .toThrow(NotFoundError);

      expect(billingPrisma.billing_customers.findFirst).toHaveBeenCalled();
      expect(billingPrisma.billing_metered_usage.create).not.toHaveBeenCalled();
    });

    it('should throw NotFoundError when subscription not found', async () => {
      const paramsWithSubscription = {
        ...validParams,
        subscriptionId: 456
      };

      billingPrisma.billing_subscriptions.findFirst.mockResolvedValue(null);

      await expect(usageService.recordUsage(paramsWithSubscription))
        .rejects
        .toThrow(NotFoundError);

      expect(billingPrisma.billing_subscriptions.findFirst).toHaveBeenCalled();
      expect(billingPrisma.billing_metered_usage.create).not.toHaveBeenCalled();
    });

    it('should throw ValidationError when required fields are missing', async () => {
      await expect(usageService.recordUsage({}))
        .rejects
        .toThrow(ValidationError);
    });

    it('should throw ValidationError when quantity is negative', async () => {
      await expect(usageService.recordUsage({ ...validParams, quantity: -1 }))
        .rejects
        .toThrow(ValidationError);
    });

    it('should throw ValidationError when unitPriceCents is negative', async () => {
      await expect(usageService.recordUsage({ ...validParams, unitPriceCents: -1 }))
        .rejects
        .toThrow(ValidationError);
    });

    it('should throw ValidationError when periodStart >= periodEnd', async () => {
      const invalidParams = {
        ...validParams,
        periodStart: new Date('2026-01-31T00:00:00Z'),
        periodEnd: new Date('2026-01-01T00:00:00Z')
      };

      await expect(usageService.recordUsage(invalidParams))
        .rejects
        .toThrow(ValidationError);
    });
  });

  describe('calculateUsageCharges', () => {
    const validParams = {
      appId: 'trashtech',
      customerId: 123,
      billingPeriodStart: new Date('2026-01-01T00:00:00Z'),
      billingPeriodEnd: new Date('2026-01-31T23:59:59Z')
    };

    const mockCustomer = {
      id: 123,
      app_id: 'trashtech'
    };

    beforeEach(() => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    });

    it('should calculate charges with no unbilled usage', async () => {
      billingPrisma.billing_metered_usage.findMany.mockResolvedValue([]);

      const result = await usageService.calculateUsageCharges(validParams);

      expect(billingPrisma.billing_customers.findFirst).toHaveBeenCalled();
      expect(billingPrisma.billing_metered_usage.findMany).toHaveBeenCalledWith({
        where: {
          app_id: validParams.appId,
          customer_id: validParams.customerId,
          billed_at: null,
          period_start: { gte: validParams.billingPeriodStart },
          period_end: { lte: validParams.billingPeriodEnd }
        },
        orderBy: [{ period_start: 'asc' }, { metric_name: 'asc' }]
      });

      expect(result).toEqual({
        appId: validParams.appId,
        customerId: validParams.customerId,
        subscriptionId: null,
        billingPeriodStart: validParams.billingPeriodStart,
        billingPeriodEnd: validParams.billingPeriodEnd,
        totalAmountCents: 0,
        usageRecords: [],
        chargesCreated: [],
        summary: 'No unbilled usage found for period'
      });
    });

    it('should calculate charges with multiple usage records', async () => {
      const mockUsageRecords = [
        {
          id: 1,
          metric_name: 'container_pickups',
          quantity: 10.5,
          unit_price_cents: 100,
          period_start: new Date('2026-01-10T00:00:00Z'),
          period_end: new Date('2026-01-20T23:59:59Z')
        },
        {
          id: 2,
          metric_name: 'container_pickups',
          quantity: 5.0,
          unit_price_cents: 100,
          period_start: new Date('2026-01-15T00:00:00Z'),
          period_end: new Date('2026-01-25T23:59:59Z')
        },
        {
          id: 3,
          metric_name: 'excess_weight',
          quantity: 50.0,
          unit_price_cents: 50, // $0.50 per kg
          period_start: new Date('2026-01-05T00:00:00Z'),
          period_end: new Date('2026-01-15T23:59:59Z')
        }
      ];

      billingPrisma.billing_metered_usage.findMany.mockResolvedValue(mockUsageRecords);

      const result = await usageService.calculateUsageCharges(validParams);

      // container_pickups: (10.5 + 5.0) * 100 = 1550 cents
      // excess_weight: 50.0 * 50 = 2500 cents
      // Total: 1550 + 2500 = 4050 cents ($40.50)
      expect(result.totalAmountCents).toBe(4050);
      expect(result.usageRecordsCount).toBe(3);
      expect(result.metrics).toHaveLength(2);

      const containerPickupsMetric = result.metrics.find(m => m.metric_name === 'container_pickups');
      expect(containerPickupsMetric.total_quantity).toBe(15.5);
      expect(containerPickupsMetric.total_amount_cents).toBe(1550);
      expect(containerPickupsMetric.records).toHaveLength(2);

      const excessWeightMetric = result.metrics.find(m => m.metric_name === 'excess_weight');
      expect(excessWeightMetric.total_quantity).toBe(50.0);
      expect(excessWeightMetric.total_amount_cents).toBe(2500);
      expect(excessWeightMetric.records).toHaveLength(1);
    });

    it('should create charges when createCharges=true', async () => {
      const mockUsageRecords = [
        {
          id: 1,
          metric_name: 'container_pickups',
          quantity: 10.0,
          unit_price_cents: 100,
          period_start: new Date('2026-01-10T00:00:00Z'),
          period_end: new Date('2026-01-20T23:59:59Z')
        }
      ];

      billingPrisma.billing_metered_usage.findMany.mockResolvedValue(mockUsageRecords);
      billingPrisma.billing_metered_usage.updateMany.mockResolvedValue({ count: 1 });

      const paramsWithCreateCharges = {
        ...validParams,
        createCharges: true
      };

      const result = await usageService.calculateUsageCharges(paramsWithCreateCharges);

      expect(billingPrisma.billing_metered_usage.updateMany).toHaveBeenCalledWith({
        where: {
          id: { in: [1] }
        },
        data: {
          billed_at: expect.any(Date)
        }
      });

      expect(result.chargesCreated).toHaveLength(1);
      expect(result.chargesCreated[0].type).toBe('usage');
      expect(result.chargesCreated[0].amount_cents).toBe(1000); // 10 * 100 = 1000 cents
    });

    it('should not create charges when total amount is zero', async () => {
      const mockUsageRecords = [
        {
          id: 1,
          metric_name: 'container_pickups',
          quantity: 0,
          unit_price_cents: 100,
          period_start: new Date('2026-01-10T00:00:00Z'),
          period_end: new Date('2026-01-20T23:59:59Z')
        }
      ];

      billingPrisma.billing_metered_usage.findMany.mockResolvedValue(mockUsageRecords);

      const paramsWithCreateCharges = {
        ...validParams,
        createCharges: true
      };

      const result = await usageService.calculateUsageCharges(paramsWithCreateCharges);

      expect(billingPrisma.billing_metered_usage.updateMany).not.toHaveBeenCalled();
      expect(result.chargesCreated).toHaveLength(0);
      expect(result.totalAmountCents).toBe(0);
    });

    it('should throw NotFoundError when customer not found', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(usageService.calculateUsageCharges(validParams))
        .rejects
        .toThrow(NotFoundError);
    });

    it('should throw ValidationError when required fields are missing', async () => {
      await expect(usageService.calculateUsageCharges({}))
        .rejects
        .toThrow(ValidationError);
    });

    it('should throw ValidationError when billingPeriodStart >= billingPeriodEnd', async () => {
      const invalidParams = {
        ...validParams,
        billingPeriodStart: new Date('2026-01-31T00:00:00Z'),
        billingPeriodEnd: new Date('2026-01-01T00:00:00Z')
      };

      await expect(usageService.calculateUsageCharges(invalidParams))
        .rejects
        .toThrow(ValidationError);
    });

    it('should handle subscription-specific usage', async () => {
      const paramsWithSubscription = {
        ...validParams,
        subscriptionId: 456
      };

      billingPrisma.billing_metered_usage.findMany.mockResolvedValue([]);

      await usageService.calculateUsageCharges(paramsWithSubscription);

      expect(billingPrisma.billing_metered_usage.findMany).toHaveBeenCalledWith({
        where: {
          app_id: validParams.appId,
          customer_id: validParams.customerId,
          billed_at: null,
          period_start: { gte: validParams.billingPeriodStart },
          period_end: { lte: validParams.billingPeriodEnd },
          subscription_id: 456
        },
        orderBy: [{ period_start: 'asc' }, { metric_name: 'asc' }]
      });
    });
  });

  describe('getUsageReport', () => {
    const validParams = {
      appId: 'trashtech',
      customerId: 123,
      startDate: new Date('2026-01-01T00:00:00Z'),
      endDate: new Date('2026-01-31T23:59:59Z')
    };

    const mockCustomer = {
      id: 123,
      app_id: 'trashtech'
    };

    beforeEach(() => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
    });

    it('should generate usage report with mixed billed/unbilled records', async () => {
      const mockUsageRecords = [
        {
          id: 1,
          metric_name: 'container_pickups',
          quantity: 10.0,
          unit_price_cents: 100,
          period_start: new Date('2026-01-10T00:00:00Z'),
          period_end: new Date('2026-01-20T23:59:59Z'),
          recorded_at: new Date('2026-01-20T10:00:00Z'),
          billed_at: new Date('2026-01-25T10:00:00Z'),
          subscription: {
            plan_name: 'Pro Plan',
            plan_id: 'pro-monthly'
          }
        },
        {
          id: 2,
          metric_name: 'container_pickups',
          quantity: 5.0,
          unit_price_cents: 100,
          period_start: new Date('2026-01-15T00:00:00Z'),
          period_end: new Date('2026-01-25T23:59:59Z'),
          recorded_at: new Date('2026-01-25T10:00:00Z'),
          billed_at: null,
          subscription: null
        }
      ];

      billingPrisma.billing_metered_usage.findMany.mockResolvedValue(mockUsageRecords);

      const result = await usageService.getUsageReport(validParams);

      expect(result.appId).toBe(validParams.appId);
      expect(result.customerId).toBe(validParams.customerId);
      expect(result.reportPeriod).toEqual({
        startDate: validParams.startDate,
        endDate: validParams.endDate
      });

      expect(result.summary.totalRecords).toBe(2);
      expect(result.summary.totalQuantity).toBe(15.0); // 10 + 5
      expect(result.summary.totalAmountCents).toBe(1500); // 10*100 + 5*100 = 1500
      expect(result.summary.billedAmountCents).toBe(1000); // Only record 1 is billed
      expect(result.summary.unbilledAmountCents).toBe(500); // Only record 2 is unbilled

      expect(result.summary.metrics).toHaveLength(1);
      const metric = result.summary.metrics[0];
      expect(metric.metric_name).toBe('container_pickups');
      expect(metric.total_quantity).toBe(15.0);
      expect(metric.total_amount_cents).toBe(1500);
      expect(metric.billed_amount_cents).toBe(1000);
      expect(metric.unbilled_amount_cents).toBe(500);

      expect(result.records).toHaveLength(2);
      expect(result.records[0].id).toBe(1);
      expect(result.records[0].billed_at).toBeDefined();
      expect(result.records[1].id).toBe(2);
      expect(result.records[1].billed_at).toBeNull();
    });

    it('should handle includeBilled/includeUnbilled filters', async () => {
      // Test includeBilled only
      billingPrisma.billing_metered_usage.findMany.mockResolvedValue([]);

      await usageService.getUsageReport({
        ...validParams,
        includeBilled: true,
        includeUnbilled: false
      });

      expect(billingPrisma.billing_metered_usage.findMany).toHaveBeenCalledWith({
        where: {
          app_id: validParams.appId,
          customer_id: validParams.customerId,
          period_start: { gte: validParams.startDate },
          period_end: { lte: validParams.endDate },
          OR: [{ billed_at: { not: null } }]
        },
        orderBy: [{ period_start: 'desc' }, { metric_name: 'asc' }],
        include: {
          subscription: {
            select: {
              plan_name: true,
              plan_id: true
            }
          }
        }
      });

      // Reset and test includeUnbilled only
      jest.clearAllMocks();
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);

      await usageService.getUsageReport({
        ...validParams,
        includeBilled: false,
        includeUnbilled: true
      });

      expect(billingPrisma.billing_metered_usage.findMany).toHaveBeenCalledWith({
        where: {
          app_id: validParams.appId,
          customer_id: validParams.customerId,
          period_start: { gte: validParams.startDate },
          period_end: { lte: validParams.endDate },
          OR: [{ billed_at: null }]
        },
        orderBy: [{ period_start: 'desc' }, { metric_name: 'asc' }],
        include: {
          subscription: {
            select: {
              plan_name: true,
              plan_id: true
            }
          }
        }
      });
    });

    it('should handle subscription-specific reports', async () => {
      const paramsWithSubscription = {
        ...validParams,
        subscriptionId: 456
      };

      billingPrisma.billing_metered_usage.findMany.mockResolvedValue([]);

      await usageService.getUsageReport(paramsWithSubscription);

      expect(billingPrisma.billing_metered_usage.findMany).toHaveBeenCalledWith({
        where: {
          app_id: validParams.appId,
          customer_id: validParams.customerId,
          period_start: { gte: validParams.startDate },
          period_end: { lte: validParams.endDate },
          subscription_id: 456
        },
        orderBy: [{ period_start: 'desc' }, { metric_name: 'asc' }],
        include: {
          subscription: {
            select: {
              plan_name: true,
              plan_id: true
            }
          }
        }
      });
    });

    it('should throw NotFoundError when customer not found', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(usageService.getUsageReport(validParams))
        .rejects
        .toThrow(NotFoundError);
    });

    it('should throw ValidationError when required fields are missing', async () => {
      await expect(usageService.getUsageReport({}))
        .rejects
        .toThrow(ValidationError);
    });

    it('should throw ValidationError when startDate >= endDate', async () => {
      const invalidParams = {
        ...validParams,
        startDate: new Date('2026-01-31T00:00:00Z'),
        endDate: new Date('2026-01-01T00:00:00Z')
      };

      await expect(usageService.getUsageReport(invalidParams))
        .rejects
        .toThrow(ValidationError);
    });
  });

  describe('markAsBilled', () => {
    const validParams = {
      appId: 'trashtech',
      usageIds: [1, 2, 3]
    };

    const mockUsageRecords = [
      { id: 1, billed_at: null },
      { id: 2, billed_at: null },
      { id: 3, billed_at: null }
    ];

    beforeEach(() => {
      billingPrisma.billing_metered_usage.findMany.mockResolvedValue(mockUsageRecords);
      billingPrisma.billing_metered_usage.updateMany.mockResolvedValue({ count: 3 });
    });

    it('should mark usage records as billed', async () => {
      const result = await usageService.markAsBilled(validParams);

      expect(billingPrisma.billing_metered_usage.findMany).toHaveBeenCalledWith({
        where: {
          id: { in: validParams.usageIds },
          app_id: validParams.appId
        },
        select: { id: true, billed_at: true }
      });

      expect(billingPrisma.billing_metered_usage.updateMany).toHaveBeenCalledWith({
        where: {
          id: { in: validParams.usageIds },
          app_id: validParams.appId
        },
        data: {
          billed_at: expect.any(Date)
        }
      });

      expect(result).toEqual({
        appId: validParams.appId,
        updatedCount: 3,
        billedAt: expect.any(Date),
        usageIds: validParams.usageIds
      });
    });

    it('should throw NotFoundError when some records not found', async () => {
      billingPrisma.billing_metered_usage.findMany.mockResolvedValue([
        { id: 1, billed_at: null },
        { id: 3, billed_at: null }
        // Missing id: 2
      ]);

      await expect(usageService.markAsBilled(validParams))
        .rejects
        .toThrow(NotFoundError);
    });

    it('should throw ValidationError when records already billed', async () => {
      billingPrisma.billing_metered_usage.findMany.mockResolvedValue([
        { id: 1, billed_at: null },
        { id: 2, billed_at: new Date('2026-01-25T10:00:00Z') }, // Already billed
        { id: 3, billed_at: null }
      ]);

      await expect(usageService.markAsBilled(validParams))
        .rejects
        .toThrow(ValidationError);
    });

    it('should throw ValidationError when usageIds is empty array', async () => {
      await expect(usageService.markAsBilled({ appId: 'trashtech', usageIds: [] }))
        .rejects
        .toThrow(ValidationError);
    });

    it('should throw ValidationError when usageIds is not an array', async () => {
      await expect(usageService.markAsBilled({ appId: 'trashtech', usageIds: 'not-an-array' }))
        .rejects
        .toThrow(ValidationError);
    });
  });
});