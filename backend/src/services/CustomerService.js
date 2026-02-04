const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError } = require('../utils/errors');

class CustomerService {
  constructor(getTilledClientFn) {
    this.getTilledClient = getTilledClientFn;
  }

  async createCustomer(appId, email, name, externalCustomerId = null, metadata = {}) {
    const tilledClient = this.getTilledClient(appId);
    const tilledCustomer = await tilledClient.createCustomer(email, name, metadata);

    return billingPrisma.billing_customers.create({
      data: {
        app_id: appId,
        external_customer_id: externalCustomerId ? String(externalCustomerId) : null,
        tilled_customer_id: tilledCustomer.id,
        email,
        name,
        metadata: metadata || {}
      }
    });
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
