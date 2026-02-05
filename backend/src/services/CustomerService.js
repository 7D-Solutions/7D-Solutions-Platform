const { billingPrisma } = require('../prisma'); // Local-first pattern pending
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError } = require('../utils/errors');

class CustomerService {
  constructor(getTilledClientFn) {
    this.getTilledClient = getTilledClientFn;
  }

  async createCustomer(appId, email, name, externalCustomerId = null, metadata = {}) {
    // Step 1: Create local pending record first (local-first pattern)
    const customerRecord = await billingPrisma.billing_customers.create({
      data: {
        app_id: appId,
        external_customer_id: externalCustomerId ? String(externalCustomerId) : null,
        tilled_customer_id: null,
        status: 'pending',
        email,
        name,
        metadata: metadata || {}
      }
    });

    // Step 2: Create customer in Tilled
    const tilledClient = this.getTilledClient(appId);

    try {
      const tilledCustomer = await tilledClient.createCustomer(email, name, metadata);

      // Step 3: Update local record with Tilled ID and active status
      const updatedCustomer = await billingPrisma.billing_customers.update({
        where: { id: customerRecord.id },
        data: {
          tilled_customer_id: tilledCustomer.id,
          status: 'active',
          updated_at: new Date()
        }
      });

      // Audit trail (fire-and-forget)
      billingPrisma.billing_events?.create({
        data: {
          app_id: appId,
          event_type: 'customer.created',
          source: 'customer_service',
          entity_type: 'customer',
          entity_id: String(updatedCustomer.id),
          payload: {
            customer_id: updatedCustomer.id,
            tilled_customer_id: tilledCustomer.id,
            external_customer_id: externalCustomerId,
            email,
          },
        },
      })?.catch(err => logger.warn('Failed to record customer audit event', { error: err.message }));

      return updatedCustomer;
    } catch (error) {
      // Step 3 (failure): Mark local record as failed
      await billingPrisma.billing_customers.update({
        where: { id: customerRecord.id },
        data: {
          status: 'failed',
          updated_at: new Date()
        }
      });

      logger.error('Customer creation failed in Tilled', {
        app_id: appId,
        customer_id: customerRecord.id,
        error_code: error.code,
        error_message: error.message,
      });

      throw error;
    }
  }

  async getCustomerById(appId, billingCustomerId) {
    const customer = await billingPrisma.billing_customers.findFirst({
      where: {
        id: billingCustomerId,
        app_id: appId
      }
    });

    if (!customer) throw new NotFoundError(`Customer ${billingCustomerId} not found for app ${appId}`);

    return customer;
  }

  async findCustomer(appId, externalCustomerId) {
    const customer = await billingPrisma.billing_customers.findFirst({
      where: {
        app_id: appId,
        external_customer_id: String(externalCustomerId)
      }
    });

    if (!customer) throw new NotFoundError(`Customer with external_customer_id ${externalCustomerId} not found for app ${appId}`);

    return customer;
  }

  async updateCustomer(appId, billingCustomerId, patch) {
    // Verify customer belongs to app
    await this.getCustomerById(appId, billingCustomerId);

    // Extract allowed fields
    const allowedFields = ['email', 'name', 'metadata'];
    const updates = {};

    allowedFields.forEach(field => {
      if (patch[field] !== undefined) {
        updates[field] = patch[field];
      }
    });

    if (Object.keys(updates).length === 0) {
      throw new ValidationError('No valid fields to update');
    }

    updates.updated_at = new Date();

    // Update in database
    const updatedCustomer = await billingPrisma.billing_customers.update({
      where: { id: billingCustomerId },
      data: updates
    });

    // Sync with Tilled if we have changes that Tilled tracks
    if (patch.email || patch.name || patch.metadata) {
      try {
        const tilledClient = this.getTilledClient(appId);
        await tilledClient.updateCustomer(updatedCustomer.tilled_customer_id, patch);
      } catch (error) {
        // CRITICAL: Log enough to reconcile later
        logger.warn('Failed to sync customer update to Tilled', {
          app_id: appId,
          billing_customer_id: billingCustomerId,
          tilled_customer_id: updatedCustomer.tilled_customer_id,
          attempted_updates: Object.keys(patch),
          error_message: error.message,
          error_code: error.code,
          // For future retry queue
          divergence_risk: patch.email ? 'high' : 'low'
        });
      }
    }

    return updatedCustomer;
  }
}

module.exports = CustomerService;
