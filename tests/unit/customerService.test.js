const CustomerService = require('../../backend/src/services/CustomerService');
const { billingPrisma } = require('../../backend/src/prisma');

jest.mock('../../backend/src/prisma', () => ({
  billingPrisma: {
    billing_customers: {
      create: jest.fn(),
      findFirst: jest.fn(),
      update: jest.fn(),
    },
  },
}));

describe('CustomerService', () => {
  let service;
  let mockTilledClient;

  beforeEach(() => {
    mockTilledClient = {
      createCustomer: jest.fn(),
      updateCustomer: jest.fn(),
    };
    service = new CustomerService(() => mockTilledClient);
    jest.clearAllMocks();
  });

  describe('createCustomer (local-first pattern)', () => {
    it('creates pending local record before calling Tilled', async () => {
      const pendingRecord = {
        id: 1,
        app_id: 'trashtech',
        email: 'test@example.com',
        name: 'Test User',
        status: 'pending',
        tilled_customer_id: null,
      };

      const activeRecord = {
        ...pendingRecord,
        status: 'active',
        tilled_customer_id: 'cus_tilled_123',
      };

      billingPrisma.billing_customers.create.mockResolvedValue(pendingRecord);
      mockTilledClient.createCustomer.mockResolvedValue({ id: 'cus_tilled_123' });
      billingPrisma.billing_customers.update.mockResolvedValue(activeRecord);

      const result = await service.createCustomer('trashtech', 'test@example.com', 'Test User');

      // Verify local record created first with pending status
      expect(billingPrisma.billing_customers.create).toHaveBeenCalledWith({
        data: expect.objectContaining({
          app_id: 'trashtech',
          email: 'test@example.com',
          name: 'Test User',
          status: 'pending',
          tilled_customer_id: null,
        }),
      });

      // Verify Tilled called after local record
      expect(mockTilledClient.createCustomer).toHaveBeenCalledWith(
        'test@example.com',
        'Test User',
        {}
      );

      // Verify local record updated to active with tilled_customer_id
      expect(billingPrisma.billing_customers.update).toHaveBeenCalledWith({
        where: { id: 1 },
        data: expect.objectContaining({
          tilled_customer_id: 'cus_tilled_123',
          status: 'active',
        }),
      });

      expect(result.status).toBe('active');
      expect(result.tilled_customer_id).toBe('cus_tilled_123');
    });

    it('marks local record as failed when Tilled call fails', async () => {
      const pendingRecord = {
        id: 2,
        app_id: 'trashtech',
        email: 'test@example.com',
        status: 'pending',
        tilled_customer_id: null,
      };

      billingPrisma.billing_customers.create.mockResolvedValue(pendingRecord);
      mockTilledClient.createCustomer.mockRejectedValue(new Error('Tilled API error'));
      billingPrisma.billing_customers.update.mockResolvedValue({
        ...pendingRecord,
        status: 'failed',
      });

      await expect(
        service.createCustomer('trashtech', 'test@example.com', 'Test User')
      ).rejects.toThrow('Tilled API error');

      // Verify pending record was created
      expect(billingPrisma.billing_customers.create).toHaveBeenCalledWith({
        data: expect.objectContaining({ status: 'pending' }),
      });

      // Verify record updated to failed
      expect(billingPrisma.billing_customers.update).toHaveBeenCalledWith({
        where: { id: 2 },
        data: expect.objectContaining({ status: 'failed' }),
      });
    });

    it('passes external_customer_id and metadata through', async () => {
      const pendingRecord = { id: 3, app_id: 'trashtech', status: 'pending' };
      const activeRecord = { ...pendingRecord, status: 'active', tilled_customer_id: 'cus_123' };

      billingPrisma.billing_customers.create.mockResolvedValue(pendingRecord);
      mockTilledClient.createCustomer.mockResolvedValue({ id: 'cus_123' });
      billingPrisma.billing_customers.update.mockResolvedValue(activeRecord);

      await service.createCustomer('trashtech', 'a@b.com', 'A', 'ext_42', { tier: 'gold' });

      expect(billingPrisma.billing_customers.create).toHaveBeenCalledWith({
        data: expect.objectContaining({
          external_customer_id: 'ext_42',
          metadata: { tier: 'gold' },
        }),
      });

      expect(mockTilledClient.createCustomer).toHaveBeenCalledWith(
        'a@b.com',
        'A',
        { tier: 'gold' }
      );
    });

    it('creates local record before Tilled call (order verification)', async () => {
      const callOrder = [];

      billingPrisma.billing_customers.create.mockImplementation(() => {
        callOrder.push('db_create');
        return Promise.resolve({ id: 1, status: 'pending' });
      });

      mockTilledClient.createCustomer.mockImplementation(() => {
        callOrder.push('tilled_create');
        return Promise.resolve({ id: 'cus_123' });
      });

      billingPrisma.billing_customers.update.mockImplementation(() => {
        callOrder.push('db_update');
        return Promise.resolve({ id: 1, status: 'active', tilled_customer_id: 'cus_123' });
      });

      await service.createCustomer('trashtech', 'test@example.com', 'Test');

      expect(callOrder).toEqual(['db_create', 'tilled_create', 'db_update']);
    });
  });

  describe('getCustomerById', () => {
    it('returns customer scoped to app', async () => {
      const customer = { id: 1, app_id: 'trashtech', email: 'test@example.com' };
      billingPrisma.billing_customers.findFirst.mockResolvedValue(customer);

      const result = await service.getCustomerById('trashtech', 1);
      expect(result).toEqual(customer);
      expect(billingPrisma.billing_customers.findFirst).toHaveBeenCalledWith({
        where: { id: 1, app_id: 'trashtech' },
      });
    });

    it('throws NotFoundError when customer not found', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(
        service.getCustomerById('trashtech', 999)
      ).rejects.toThrow('Customer 999 not found for app trashtech');
    });
  });

  describe('findCustomer', () => {
    it('finds customer by external_customer_id scoped to app', async () => {
      const customer = { id: 1, app_id: 'trashtech', external_customer_id: '42' };
      billingPrisma.billing_customers.findFirst.mockResolvedValue(customer);

      const result = await service.findCustomer('trashtech', 42);
      expect(result).toEqual(customer);
    });

    it('throws NotFoundError when not found', async () => {
      billingPrisma.billing_customers.findFirst.mockResolvedValue(null);

      await expect(
        service.findCustomer('trashtech', 'ext_missing')
      ).rejects.toThrow('not found for app trashtech');
    });
  });

  describe('updateCustomer', () => {
    it('updates allowed fields and syncs to Tilled', async () => {
      const customer = {
        id: 1,
        app_id: 'trashtech',
        tilled_customer_id: 'cus_123',
      };

      billingPrisma.billing_customers.findFirst.mockResolvedValue(customer);
      billingPrisma.billing_customers.update.mockResolvedValue({
        ...customer,
        email: 'new@example.com',
      });
      mockTilledClient.updateCustomer.mockResolvedValue({});

      const result = await service.updateCustomer('trashtech', 1, { email: 'new@example.com' });

      expect(billingPrisma.billing_customers.update).toHaveBeenCalledWith({
        where: { id: 1 },
        data: expect.objectContaining({ email: 'new@example.com' }),
      });

      expect(mockTilledClient.updateCustomer).toHaveBeenCalledWith('cus_123', { email: 'new@example.com' });
    });

    it('throws ValidationError when no valid fields provided', async () => {
      const customer = { id: 1, app_id: 'trashtech' };
      billingPrisma.billing_customers.findFirst.mockResolvedValue(customer);

      await expect(
        service.updateCustomer('trashtech', 1, { invalid_field: 'value' })
      ).rejects.toThrow('No valid fields to update');
    });
  });
});
