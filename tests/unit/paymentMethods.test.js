const BillingService = require('../../backend/src/billingService');
const { billingPrisma } = require('../../backend/src/prisma');
const TilledClient = require('../../backend/src/tilledClient');

// Mock dependencies
jest.mock('../../backend/src/tilledClient');
jest.mock('../../backend/src/prisma', () => ({
  billingPrisma: {
    billing_customers: {
      findFirst: jest.fn(),
      update: jest.fn()
    },
    billing_payment_methods: {
      findMany: jest.fn(),
      findFirst: jest.fn(),
      upsert: jest.fn(),
      update: jest.fn(),
      updateMany: jest.fn()
    },
    $transaction: jest.fn()
  }
}));

describe('BillingService payment methods', () => {
  let service;
  let mockTilledClient;

  beforeEach(() => {
    service = new BillingService();
    mockTilledClient = {
      attachPaymentMethod: jest.fn(),
      getPaymentMethod: jest.fn(),
      detachPaymentMethod: jest.fn()
    };
    TilledClient.mockImplementation(() => mockTilledClient);
    jest.clearAllMocks();
  });

  describe('listPaymentMethods', () => {
    it('returns only non-deleted methods scoped to app/customer', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech',
        tilled_customer_id: 'cus_test'
      };

      const mockMethods = [
        {
          tilled_payment_method_id: 'pm_1',
          type: 'card',
          brand: 'visa',
          last4: '4242',
          exp_month: 12,
          exp_year: 2028,
          is_default: true,
          deleted_at: null
        },
        {
          tilled_payment_method_id: 'pm_2',
          type: 'card',
          brand: 'mastercard',
          last4: '5555',
          exp_month: 6,
          exp_year: 2027,
          is_default: false,
          deleted_at: null
        }
      ];

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_payment_methods.findMany.mockResolvedValue(mockMethods);

      const result = await service.listPaymentMethods('trashtech', 1);

      expect(billingPrisma.billing_customers.findFirst).toHaveBeenCalledWith({
        where: { id: 1, app_id: 'trashtech' }
      });

      expect(billingPrisma.billing_payment_methods.findMany).toHaveBeenCalledWith({
        where: {
          app_id: 'trashtech',
          billing_customer_id: 1,
          deleted_at: null
        },
        orderBy: [
          { is_default: 'desc' },
          { created_at: 'desc' }
        ]
      });

      expect(result.billing_customer_id).toBe(1);
      expect(result.payment_methods).toHaveLength(2);
      expect(result.payment_methods[0].tilled_payment_method_id).toBe('pm_1');
      expect(result.payment_methods[0].is_default).toBe(true);
    });

    it('excludes deleted payment methods', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech'
      };

      const mockMethods = [
        {
          tilled_payment_method_id: 'pm_1',
          type: 'card',
          deleted_at: null
        }
      ];

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_payment_methods.findMany.mockResolvedValue(mockMethods);

      await service.listPaymentMethods('trashtech', 1);

      expect(billingPrisma.billing_payment_methods.findMany).toHaveBeenCalledWith({
        where: {
          app_id: 'trashtech',
          billing_customer_id: 1,
          deleted_at: null  // Critical: excludes deleted
        },
        orderBy: [
          { is_default: 'desc' },
          { created_at: 'desc' }
        ]
      });
    });

    it('throws 404 when customer not in app scope', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(
        service.listPaymentMethods('trashtech', 999)
      ).rejects.toThrow('Customer 999 not found for app trashtech');
    });
  });

  describe('addPaymentMethod', () => {
    it('attaches to Tilled and upserts local masked record', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech',
        tilled_customer_id: 'cus_test'
      };

      const mockTilledPM = {
        id: 'pm_new',
        type: 'card',
        card: {
          brand: 'visa',
          last4: '4242',
          exp_month: 12,
          exp_year: 2028
        }
      };

      const mockUpsertedPM = {
        id: 1,
        tilled_payment_method_id: 'pm_new',
        type: 'card',
        brand: 'visa',
        last4: '4242',
        exp_month: 12,
        exp_year: 2028,
        is_default: false,
        billing_customer_id: 1
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      mockTilledClient.attachPaymentMethod.mockResolvedValue({ id: 'pm_new' });
      mockTilledClient.getPaymentMethod.mockResolvedValue(mockTilledPM);
      billingPrisma.billing_payment_methods.upsert.mockResolvedValue(mockUpsertedPM);

      const result = await service.addPaymentMethod('trashtech', 1, 'pm_new', 'card', {});

      expect(mockTilledClient.attachPaymentMethod).toHaveBeenCalledWith(
        'pm_new',
        'cus_test'
      );

      expect(mockTilledClient.getPaymentMethod).toHaveBeenCalledWith('pm_new');

      expect(billingPrisma.billing_payment_methods.upsert).toHaveBeenCalledWith({
        where: { tilled_payment_method_id: 'pm_new' },
        create: expect.objectContaining({
          app_id: 'trashtech',
          billing_customer_id: 1,
          tilled_payment_method_id: 'pm_new',
          type: 'card',
          brand: 'visa',
          last4: '4242',
          exp_month: 12,
          exp_year: 2028
        }),
        update: expect.objectContaining({
          brand: 'visa',
          last4: '4242',
          deleted_at: null
        })
      });

      expect(result.tilled_payment_method_id).toBe('pm_new');
    });

    it('handles ACH payment methods correctly', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech',
        tilled_customer_id: 'cus_test'
      };

      const mockTilledPM = {
        id: 'pm_ach',
        type: 'ach_debit',
        ach_debit: {
          bank_name: 'Chase',
          last4: '6789'
        }
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      mockTilledClient.attachPaymentMethod.mockResolvedValue({ id: 'pm_ach' });
      mockTilledClient.getPaymentMethod.mockResolvedValue(mockTilledPM);
      billingPrisma.billing_payment_methods.upsert.mockResolvedValue({
        tilled_payment_method_id: 'pm_ach',
        type: 'ach_debit',
        bank_name: 'Chase',
        bank_last4: '6789'
      });

      const result = await service.addPaymentMethod('trashtech', 1, 'pm_ach', 'ach_debit', {});

      expect(billingPrisma.billing_payment_methods.upsert).toHaveBeenCalledWith({
        where: { tilled_payment_method_id: 'pm_ach' },
        create: expect.objectContaining({
          type: 'ach_debit',
          bank_name: 'Chase',
          bank_last4: '6789'
        }),
        update: expect.anything()
      });

      expect(result.type).toBe('ach_debit');
    });

    it('continues if getPaymentMethod fails (stores minimal)', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech',
        tilled_customer_id: 'cus_test'
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      mockTilledClient.attachPaymentMethod.mockResolvedValue({ id: 'pm_new' });
      mockTilledClient.getPaymentMethod.mockRejectedValue(new Error('Tilled API error'));
      billingPrisma.billing_payment_methods.upsert.mockResolvedValue({
        tilled_payment_method_id: 'pm_new',
        type: 'card'
      });

      const result = await service.addPaymentMethod('trashtech', 1, 'pm_new', 'card', {});

      // Should still complete despite getPaymentMethod failure
      expect(result.tilled_payment_method_id).toBe('pm_new');
      expect(billingPrisma.billing_payment_methods.upsert).toHaveBeenCalled();
    });

    it('throws 404 when customer not in app scope', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(
        service.addPaymentMethod('trashtech', 999, 'pm_new', 'card', {})
      ).rejects.toThrow('Customer 999 not found for app trashtech');
    });
  });

  describe('setDefaultPaymentMethodById', () => {
    it('sets one default and updates customer fast-path', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech',
        tilled_customer_id: 'cus_test',
        default_payment_method_id: 'pm_old',
        payment_method_type: 'card'
      };

      const mockPM = {
        tilled_payment_method_id: 'pm_new',
        type: 'card',
        is_default: false,
        deleted_at: null
      };

      const mockUpdatedPM = {
        ...mockPM,
        is_default: true
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(mockPM);

      // Mock transaction to execute callback with transaction object
      billingPrisma.$transaction.mockImplementation(async (callback) => {
        return callback({
          billing_payment_methods: {
            updateMany: jest.fn().mockResolvedValue({ count: 2 }),
            update: jest.fn().mockResolvedValue(mockUpdatedPM),
            findFirst: jest.fn().mockResolvedValue(mockUpdatedPM)
          },
          billing_customers: {
            update: jest.fn().mockResolvedValue({
              ...mockCustomer,
              default_payment_method_id: 'pm_new'
            })
          }
        });
      });

      const result = await service.setDefaultPaymentMethodById('trashtech', 1, 'pm_new');

      // Verify transaction was used
      expect(billingPrisma.$transaction).toHaveBeenCalled();

      // Verify result
      expect(result.tilled_payment_method_id).toBe('pm_new');
      expect(result.is_default).toBe(true);
    });

    it('throws 404 when payment method not found', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech'
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(null);

      await expect(
        service.setDefaultPaymentMethodById('trashtech', 1, 'pm_missing', 'card')
      ).rejects.toThrow('Payment method pm_missing not found for customer 1');
    });

    it('throws when payment method is deleted', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech'
      };

      // Deleted PMs are filtered out by deleted_at: null, so findFirst returns null
      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(null);

      await expect(
        service.setDefaultPaymentMethodById('trashtech', 1, 'pm_deleted')
      ).rejects.toThrow('Payment method pm_deleted not found for customer 1');
    });

    it('throws 404 when customer not in app scope', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(
        service.setDefaultPaymentMethodById('trashtech', 999, 'pm_new', 'card')
      ).rejects.toThrow('Customer 999 not found for app trashtech');
    });
  });

  describe('deletePaymentMethod', () => {
    it('soft-deletes and clears default if deleting default', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech',
        tilled_customer_id: 'cus_test',
        default_payment_method_id: 'pm_delete',
        payment_method_type: 'card'
      };

      const mockPM = {
        id: 123,
        tilled_payment_method_id: 'pm_delete',
        billing_customer_id: 1,
        is_default: true,
        deleted_at: null
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(mockPM);
      billingPrisma.billing_payment_methods.update.mockResolvedValue({
        ...mockPM,
        deleted_at: new Date()
      });
      billingPrisma.billing_customers.update.mockResolvedValue({
        ...mockCustomer,
        default_payment_method_id: null,
        payment_method_type: null
      });
      mockTilledClient.detachPaymentMethod.mockResolvedValue({});

      const result = await service.deletePaymentMethod('trashtech', 1, 'pm_delete');

      // Verify soft delete (using verified record id to avoid TOCTOU race)
      expect(billingPrisma.billing_payment_methods.update).toHaveBeenCalledWith({
        where: { id: 123 },
        data: { deleted_at: expect.any(Date), is_default: false }
      });

      // Verify default cleared from customer
      expect(billingPrisma.billing_customers.update).toHaveBeenCalledWith({
        where: { id: 1 },
        data: {
          default_payment_method_id: null,
          payment_method_type: null,
          updated_at: expect.any(Date)
        }
      });

      // Verify Tilled detach called
      expect(mockTilledClient.detachPaymentMethod).toHaveBeenCalledWith('pm_delete');

      expect(result.deleted).toBe(true);
    });

    it('does not clear customer default when deleting non-default PM', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech',
        default_payment_method_id: 'pm_default'
      };

      const mockPM = {
        tilled_payment_method_id: 'pm_other',
        billing_customer_id: 1,
        is_default: false,
        deleted_at: null
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(mockPM);
      billingPrisma.billing_payment_methods.update.mockResolvedValue(mockPM);
      mockTilledClient.detachPaymentMethod.mockResolvedValue({});

      await service.deletePaymentMethod('trashtech', 1, 'pm_other');

      // Should NOT update customer
      expect(billingPrisma.billing_customers.update).not.toHaveBeenCalled();
    });

    it('continues if Tilled detach fails (warn only)', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech'
      };

      const mockPM = {
        tilled_payment_method_id: 'pm_delete',
        billing_customer_id: 1,
        is_default: false,
        deleted_at: null
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(mockPM);
      billingPrisma.billing_payment_methods.update.mockResolvedValue(mockPM);
      mockTilledClient.detachPaymentMethod.mockRejectedValue(new Error('Tilled API error'));

      const result = await service.deletePaymentMethod('trashtech', 1, 'pm_delete');

      // Should still complete
      expect(result.deleted).toBe(true);
    });

    it('throws 404 when payment method not found', async () => {
      const mockCustomer = {
        id: 1,
        app_id: 'trashtech'
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(mockCustomer);
      billingPrisma.billing_payment_methods.findFirst.mockResolvedValue(null);

      await expect(
        service.deletePaymentMethod('trashtech', 1, 'pm_missing')
      ).rejects.toThrow('Payment method pm_missing not found for customer 1');
    });

    it('throws 404 when customer not in app scope', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(
        service.deletePaymentMethod('trashtech', 999, 'pm_delete')
      ).rejects.toThrow('Customer 999 not found for app trashtech');
    });
  });
});
