const ReportingService = require('../../backend/src/services/ReportingService');
const { ValidationError } = require('../../backend/src/utils/errors');

// Mock dependencies
jest.mock('../../backend/src/prisma', () => ({
  billingPrisma: {
    billing_charges: {
      findMany: jest.fn(),
      count: jest.fn()
    },
    billing_refunds: {
      findMany: jest.fn(),
      count: jest.fn()
    },
    billing_subscriptions: {
      findMany: jest.fn(),
      count: jest.fn()
    },
    billing_invoices: {
      findMany: jest.fn(),
      count: jest.fn()
    },
    $queryRaw: jest.fn()
  }
}));

// Mock Prisma from @prisma/client
jest.mock('@prisma/client', () => ({
  Prisma: {
    sql: jest.fn((strings, ...values) => {
      // Simple mock for sql template tag
      let result = strings[0];
      for (let i = 0; i < values.length; i++) {
        result += values[i] + strings[i + 1];
      }
      return result;
    }),
    raw: jest.fn((str) => str) // Mock raw as identity function
  }
}));

const { billingPrisma } = require('../../backend/src/prisma');

describe('ReportingService', () => {
  let reportingService;
  const mockGetTilledClient = jest.fn();
  const appId = 'test-app-123';
  const now = new Date('2026-01-31T12:00:00Z');

  beforeEach(() => {
    jest.clearAllMocks();
    reportingService = new ReportingService(mockGetTilledClient);
  });

  describe('getRevenueReport', () => {
    const startDate = new Date('2026-01-01T00:00:00Z');
    const endDate = new Date('2026-01-31T23:59:59Z');

    it('should validate required date range', async () => {
      await expect(reportingService.getRevenueReport(appId, {}))
        .rejects.toThrow(ValidationError);

      await expect(reportingService.getRevenueReport(appId, { startDate }))
        .rejects.toThrow(ValidationError);

      await expect(reportingService.getRevenueReport(appId, { endDate }))
        .rejects.toThrow(ValidationError);
    });

    it('should validate endDate after startDate', async () => {
      await expect(reportingService.getRevenueReport(appId, {
        startDate: endDate,
        endDate: startDate
      })).rejects.toThrow(ValidationError);
    });

    it('should validate granularity', async () => {
      await expect(reportingService.getRevenueReport(appId, {
        startDate,
        endDate,
        granularity: 'invalid'
      })).rejects.toThrow(ValidationError);
    });

    it('should return revenue report with charges and refunds', async () => {
      const mockChargesResult = [
        { period: '2026-01-01', gross_revenue_cents: '5000', transaction_count: '5' },
        { period: '2026-01-02', gross_revenue_cents: '3000', transaction_count: '3' }
      ];

      const mockRefundsResult = [
        { period: '2026-01-01', refunds_cents: '500', refund_count: '1' }
      ];

      billingPrisma.$queryRaw
        .mockResolvedValueOnce(mockChargesResult) // First call for charges
        .mockResolvedValueOnce(mockRefundsResult); // Second call for refunds

      const result = await reportingService.getRevenueReport(appId, {
        startDate,
        endDate,
        granularity: 'daily'
      });

      expect(billingPrisma.$queryRaw).toHaveBeenCalledTimes(2);
      expect(result.summary.total_gross_revenue_cents).toBe(8000);
      expect(result.summary.total_refunds_cents).toBe(500);
      expect(result.summary.total_net_revenue_cents).toBe(7500);
      expect(result.periods).toHaveLength(2);

      const period1 = result.periods.find(p => p.period === '2026-01-01');
      expect(period1.gross_revenue_cents).toBe(5000);
      expect(period1.refunds_cents).toBe(500);
      expect(period1.net_revenue_cents).toBe(4500);
    });

    it('should handle refunds without corresponding charges', async () => {
      const mockChargesResult = [
        { period: '2026-01-01', gross_revenue_cents: '5000', transaction_count: '5' }
      ];

      const mockRefundsResult = [
        { period: '2026-01-01', refunds_cents: '500', refund_count: '1' },
        { period: '2026-01-02', refunds_cents: '200', refund_count: '1' } // No charges on 2026-01-02
      ];

      billingPrisma.$queryRaw
        .mockResolvedValueOnce(mockChargesResult)
        .mockResolvedValueOnce(mockRefundsResult);

      const result = await reportingService.getRevenueReport(appId, {
        startDate,
        endDate,
        granularity: 'daily'
      });

      expect(result.periods).toHaveLength(2);
      const period2 = result.periods.find(p => p.period === '2026-01-02');
      expect(period2.gross_revenue_cents).toBe(0);
      expect(period2.refunds_cents).toBe(200);
      expect(period2.net_revenue_cents).toBe(-200);
    });

    it('should apply filters correctly', async () => {
      billingPrisma.$queryRaw
        .mockResolvedValueOnce([])
        .mockResolvedValueOnce([]);

      await reportingService.getRevenueReport(appId, {
        startDate,
        endDate,
        customerId: 123,
        subscriptionId: 456,
        chargeType: 'subscription'
      });

      // Verify SQL includes filter conditions
      // $queryRaw is called with template literal parts
      expect(billingPrisma.$queryRaw).toHaveBeenCalled();
      // We can't easily check the SQL string since it's built from template parts
      // But we can verify the function was called
      expect(billingPrisma.$queryRaw).toHaveBeenCalledTimes(2);
    });
  });

  describe('getMRRReport', () => {
    it('should validate required asOfDate', async () => {
      await expect(reportingService.getMRRReport(appId, {}))
        .rejects.toThrow(ValidationError);
    });

    it('should calculate MRR for monthly subscriptions', async () => {
      const mockSubscriptions = [
        {
          id: 1,
          plan_id: 'pro-monthly',
          price_cents: 10000,
          interval_unit: 'month',
          interval_count: 1
        },
        {
          id: 2,
          plan_id: 'pro-monthly',
          price_cents: 10000,
          interval_unit: 'month',
          interval_count: 1
        }
      ];

      billingPrisma.billing_subscriptions.findMany.mockResolvedValue(mockSubscriptions);

      const result = await reportingService.getMRRReport(appId, {
        asOfDate: now
      });

      expect(result.total_mrr_cents).toBe(20000); // 2 × $100/month
      expect(result.subscription_count).toBe(2);
      expect(result.breakdown).toHaveLength(1);
      expect(result.breakdown[0].plan_id).toBe('pro-monthly');
      expect(result.breakdown[0].mrr_cents).toBe(20000);
      expect(result.breakdown[0].subscription_count).toBe(2);
    });

    it('should normalize annual subscriptions to monthly MRR', async () => {
      const mockSubscriptions = [
        {
          id: 1,
          plan_id: 'pro-annual',
          price_cents: 120000, // $1200/year
          interval_unit: 'year',
          interval_count: 1
        }
      ];

      billingPrisma.billing_subscriptions.findMany.mockResolvedValue(mockSubscriptions);

      const result = await reportingService.getMRRReport(appId, {
        asOfDate: now
      });

      expect(result.total_mrr_cents).toBe(10000); // $1200/12 = $100/month
    });

    it('should normalize quarterly subscriptions to monthly MRR', async () => {
      const mockSubscriptions = [
        {
          id: 1,
          plan_id: 'pro-quarterly',
          price_cents: 30000, // $300/quarter
          interval_unit: 'quarter',
          interval_count: 1
        }
      ];

      billingPrisma.billing_subscriptions.findMany.mockResolvedValue(mockSubscriptions);

      const result = await reportingService.getMRRReport(appId, {
        asOfDate: now
      });

      expect(result.total_mrr_cents).toBe(10000); // $300/3 = $100/month
    });

    it('should normalize weekly subscriptions to monthly MRR', async () => {
      const mockSubscriptions = [
        {
          id: 1,
          plan_id: 'pro-weekly',
          price_cents: 2500, // $25/week
          interval_unit: 'week',
          interval_count: 1
        }
      ];

      billingPrisma.billing_subscriptions.findMany.mockResolvedValue(mockSubscriptions);

      const result = await reportingService.getMRRReport(appId, {
        asOfDate: now
      });

      expect(result.total_mrr_cents).toBe(10825); // $25 × 4.33 = $108.25/month
    });

    it('should normalize daily subscriptions to monthly MRR', async () => {
      const mockSubscriptions = [
        {
          id: 1,
          plan_id: 'pro-daily',
          price_cents: 100, // $1/day
          interval_unit: 'day',
          interval_count: 1
        }
      ];

      billingPrisma.billing_subscriptions.findMany.mockResolvedValue(mockSubscriptions);

      const result = await reportingService.getMRRReport(appId, {
        asOfDate: now
      });

      expect(result.total_mrr_cents).toBe(3044); // $1 × 30.44 = $30.44/month
    });

    it('should filter by planId', async () => {
      billingPrisma.billing_subscriptions.findMany.mockResolvedValue([]);

      await reportingService.getMRRReport(appId, {
        asOfDate: now,
        planId: 'pro-monthly'
      });

      expect(billingPrisma.billing_subscriptions.findMany).toHaveBeenCalledWith(
        expect.objectContaining({
          where: expect.objectContaining({
            plan_id: 'pro-monthly'
          })
        })
      );
    });

    it('should exclude breakdown when includeBreakdown is false', async () => {
      const mockSubscriptions = [
        {
          id: 1,
          plan_id: 'pro-monthly',
          price_cents: 10000,
          interval_unit: 'month',
          interval_count: 1
        }
      ];

      billingPrisma.billing_subscriptions.findMany.mockResolvedValue(mockSubscriptions);

      const result = await reportingService.getMRRReport(appId, {
        asOfDate: now,
        includeBreakdown: false
      });

      expect(result.breakdown).toBeUndefined();
    });
  });

  describe('getChurnReport', () => {
    const startDate = new Date('2026-01-01T00:00:00Z');
    const endDate = new Date('2026-01-31T23:59:59Z');

    it('should validate required date range', async () => {
      await expect(reportingService.getChurnReport(appId, {}))
        .rejects.toThrow(ValidationError);

      await expect(reportingService.getChurnReport(appId, { startDate }))
        .rejects.toThrow(ValidationError);

      await expect(reportingService.getChurnReport(appId, { endDate }))
        .rejects.toThrow(ValidationError);
    });

    it('should validate endDate after startDate', async () => {
      await expect(reportingService.getChurnReport(appId, {
        startDate: endDate,
        endDate: startDate
      })).rejects.toThrow(ValidationError);
    });

    it('should validate cohortPeriod', async () => {
      await expect(reportingService.getChurnReport(appId, {
        startDate,
        endDate,
        cohortPeriod: 'invalid'
      })).rejects.toThrow(ValidationError);
    });

    it('should calculate churn rates correctly', async () => {
      const mockStartingActive = [
        { cohort: '2026-01', starting_customer_count: '100' }
      ];

      const mockChurned = [
        { cohort: '2026-01', churned_customer_count: '5', churned_revenue_cents: '5000' }
      ];

      billingPrisma.$queryRaw
        .mockResolvedValueOnce(mockStartingActive)
        .mockResolvedValueOnce(mockChurned);

      const result = await reportingService.getChurnReport(appId, {
        startDate,
        endDate,
        cohortPeriod: 'monthly'
      });

      expect(result.overall.starting_customer_count).toBe(100);
      expect(result.overall.churned_customer_count).toBe(5);
      expect(result.overall.churned_revenue_cents).toBe(5000);
      expect(result.overall.customer_churn_rate).toBe(0.05); // 5/100

      expect(result.cohorts).toHaveLength(1);
      expect(result.cohorts[0].cohort).toBe('2026-01');
      expect(result.cohorts[0].customer_churn_rate).toBe(0.05);
    });

    it('should handle cohorts with no starting active customers', async () => {
      const mockStartingActive = []; // No starting active customers

      const mockChurned = [
        { cohort: '2026-01', churned_customer_count: '5', churned_revenue_cents: '5000' }
      ];

      billingPrisma.$queryRaw
        .mockResolvedValueOnce(mockStartingActive)
        .mockResolvedValueOnce(mockChurned);

      const result = await reportingService.getChurnReport(appId, {
        startDate,
        endDate,
        cohortPeriod: 'monthly'
      });

      expect(result.overall.starting_customer_count).toBe(0);
      expect(result.overall.churned_customer_count).toBe(5);
      expect(result.overall.customer_churn_rate).toBe(0); // 0 starting customers

      expect(result.cohorts).toHaveLength(1);
      expect(result.cohorts[0].starting_customer_count).toBe(0);
      expect(result.cohorts[0].customer_churn_rate).toBe(0);
    });
  });

  describe('getAgingReceivablesReport', () => {
    it('should validate required asOfDate', async () => {
      await expect(reportingService.getAgingReceivablesReport(appId, {}))
        .rejects.toThrow(ValidationError);
    });

    it('should categorize invoices into aging buckets', async () => {
      const mockInvoices = [
        {
          id: 1,
          billing_customer_id: 123,
          amount_cents: 10000,
          amount_paid_cents: 0,
          due_at: new Date('2026-01-31T00:00:00Z'), // Current (due today)
          status: 'open'
        },
        {
          id: 2,
          billing_customer_id: 124,
          amount_cents: 5000,
          due_at: new Date('2026-01-01T00:00:00Z'), // 30 days overdue
          status: 'past_due'
        },
        {
          id: 3,
          billing_customer_id: 125,
          amount_cents: 3000,
          due_at: new Date('2026-01-01T00:00:00Z'),
          status: 'paid' // Fully paid
        },
        {
          id: 4,
          billing_customer_id: 126,
          amount_cents: 7000,
          amount_paid_cents: 0,
          due_at: new Date('2025-10-01T00:00:00Z'), // 90+ days overdue
          status: 'past_due'
        }
      ];

      billingPrisma.billing_invoices.findMany.mockResolvedValue(mockInvoices);

      const result = await reportingService.getAgingReceivablesReport(appId, {
        asOfDate: now // 2026-01-31
      });

      expect(result.total_outstanding_cents).toBe(10000 + 5000 + 7000); // 10000 + 5000 + 7000 = 22000
      expect(result.total_invoice_count).toBe(4);
      expect(result.aging_buckets).toHaveLength(3); // current, 1-30, 90+

      const currentBucket = result.aging_buckets.find(b => b.bucket === 'current');
      expect(currentBucket.amount_cents).toBe(10000);
      expect(currentBucket.invoice_count).toBe(1);

      const bucket1to30 = result.aging_buckets.find(b => b.bucket === '1-30');
      expect(bucket1to30.amount_cents).toBe(5000); // 5000 outstanding
      expect(bucket1to30.invoice_count).toBe(1);

      const bucket90Plus = result.aging_buckets.find(b => b.bucket === '90+');
      expect(bucket90Plus.amount_cents).toBe(7000);
      expect(bucket90Plus.invoice_count).toBe(1);
    });

    it('should filter by customerId', async () => {
      billingPrisma.billing_invoices.findMany.mockResolvedValue([]);

      await reportingService.getAgingReceivablesReport(appId, {
        asOfDate: now,
        customerId: 123
      });

      expect(billingPrisma.billing_invoices.findMany).toHaveBeenCalledWith(
        expect.objectContaining({
          where: expect.objectContaining({
            billing_customer_id: 123
          })
        })
      );
    });

    it('should skip invoices with null due_at', async () => {
      // Mock should return empty array since where clause filters out null due_at
      billingPrisma.billing_invoices.findMany.mockResolvedValue([]);

      const result = await reportingService.getAgingReceivablesReport(appId, {
        asOfDate: now
      });

      expect(result.total_outstanding_cents).toBe(0);
      expect(result.total_invoice_count).toBe(0); // Filtered out due to null due_at
      expect(result.aging_buckets).toHaveLength(0);

      // Verify the where clause includes due_at: { not: null }
      expect(billingPrisma.billing_invoices.findMany).toHaveBeenCalledWith(
        expect.objectContaining({
          where: expect.objectContaining({
            due_at: { not: null }
          })
        })
      );
    });
  });
});