const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError, ConflictError, UnauthorizedError } = require('../utils/errors');

class PaymentMethodService {
  constructor(getTilledClientFn, customerService) {
    this.getTilledClient = getTilledClientFn;
    this.customerService = customerService;
  }

  async setDefaultPaymentMethod(appId, customerId, paymentMethodId, paymentMethodType) {
    // Verify customer belongs to app
    const customer = await this.customerService.getCustomerById(appId, customerId);

    return billingPrisma.billing_customers.update({
      where: { id: customerId },
      data: {
        default_payment_method_id: paymentMethodId,
        payment_method_type: paymentMethodType,
        updated_at: new Date()
      }
    });
  }

  async listPaymentMethods(appId, billingCustomerId) {
    // Verify customer belongs to app
    await this.customerService.getCustomerById(appId, billingCustomerId);

    const paymentMethods = await billingPrisma.billing_payment_methods.findMany({
      where: {
        app_id: appId,
        billing_customer_id: billingCustomerId,
        deleted_at: null,
        status: 'active'
      },
      orderBy: [
        { is_default: 'desc' },
        { created_at: 'desc' }
      ]
    });

    return {
      billing_customer_id: billingCustomerId,
      payment_methods: paymentMethods
    };
  }

  async addPaymentMethod(appId, billingCustomerId, paymentMethodId) {
    // Verify customer belongs to app
    const customer = await this.customerService.getCustomerById(appId, billingCustomerId);

    // Step 1: Create local pending record first (local-first pattern)
    // We already have the tilled_payment_method_id from client-side tokenization
    const pmRecord = await billingPrisma.billing_payment_methods.upsert({
      where: { tilled_payment_method_id: paymentMethodId },
      update: {
        app_id: appId,
        billing_customer_id: billingCustomerId,
        tilled_payment_method_id: paymentMethodId,
        type: 'card',
        status: 'pending',
        deleted_at: null,
        updated_at: new Date()
      },
      create: {
        app_id: appId,
        billing_customer_id: billingCustomerId,
        tilled_payment_method_id: paymentMethodId,
        type: 'card',
        status: 'pending',
        is_default: false,
        metadata: {},
        created_at: new Date(),
        updated_at: new Date()
      }
    });

    // Step 2: Attach to Tilled customer
    const tilledClient = this.getTilledClient(appId);

    try {
      await tilledClient.attachPaymentMethod(paymentMethodId, customer.tilled_customer_id);
    } catch (error) {
      // Tilled attach failed — mark as failed and soft-delete the local record
      await billingPrisma.billing_payment_methods.update({
        where: { id: pmRecord.id },
        data: { status: 'failed', deleted_at: new Date() }
      });

      logger.error('Failed to attach payment method in Tilled', {
        app_id: appId,
        billing_customer_id: billingCustomerId,
        tilled_payment_method_id: paymentMethodId,
        error_code: error.code,
        error_message: error.message,
      });

      throw error;
    }

    // Step 3: Fetch full payment method details from Tilled (best effort)
    let tilledPM;
    try {
      tilledPM = await tilledClient.getPaymentMethod(paymentMethodId);
    } catch (error) {
      logger.warn('Failed to fetch payment method details from Tilled', {
        app_id: appId,
        billing_customer_id: billingCustomerId,
        tilled_payment_method_id: paymentMethodId,
        error_message: error.message
      });
      // Fallback to minimal data
      tilledPM = { id: paymentMethodId, type: 'card' };
    }

    // Step 4: Update local record with full details from Tilled
    const pmData = {
      status: 'active',
      type: tilledPM.type,
      brand: tilledPM.card?.brand || null,
      last4: tilledPM.card?.last4 || tilledPM.ach_debit?.last4 || tilledPM.eft_debit?.last4 || null,
      exp_month: tilledPM.card?.exp_month || null,
      exp_year: tilledPM.card?.exp_year || null,
      bank_name: tilledPM.ach_debit?.bank_name || tilledPM.eft_debit?.bank_name || null,
      bank_last4: tilledPM.ach_debit?.last4 || tilledPM.eft_debit?.last4 || null,
      metadata: tilledPM.metadata || {},
      updated_at: new Date()
    };

    return billingPrisma.billing_payment_methods.update({
      where: { id: pmRecord.id },
      data: pmData
    });
  }

  async setDefaultPaymentMethodById(appId, billingCustomerId, tilledPaymentMethodId) {
    // Verify customer belongs to app
    const customer = await this.customerService.getCustomerById(appId, billingCustomerId);

    // Verify payment method exists and belongs to customer
    const paymentMethod = await billingPrisma.billing_payment_methods.findFirst({
      where: {
        tilled_payment_method_id: tilledPaymentMethodId,
        billing_customer_id: billingCustomerId,
        app_id: appId,
        deleted_at: null,
        status: 'active'
      }
    });

    if (!paymentMethod) {
      throw new NotFoundError(`Payment method ${tilledPaymentMethodId} not found for customer ${billingCustomerId}`);
    }

    // Use transaction to ensure atomicity
    return billingPrisma.$transaction(async (tx) => {
      // Clear all other defaults for this customer
      await tx.billing_payment_methods.updateMany({
        where: {
          billing_customer_id: billingCustomerId,
          app_id: appId
        },
        data: {
          is_default: false,
          updated_at: new Date()
        }
      });

      // Set this one as default
      await tx.billing_payment_methods.update({
        where: { tilled_payment_method_id: tilledPaymentMethodId },
        data: {
          is_default: true,
          updated_at: new Date()
        }
      });

      // Update customer fast-path
      await tx.billing_customers.update({
        where: { id: billingCustomerId },
        data: {
          default_payment_method_id: tilledPaymentMethodId,
          payment_method_type: paymentMethod.type,
          updated_at: new Date()
        }
      });

      return tx.billing_payment_methods.findFirst({
        where: { tilled_payment_method_id: tilledPaymentMethodId }
      });
    });
  }

  async deletePaymentMethod(appId, billingCustomerId, tilledPaymentMethodId) {
    // Verify customer belongs to app
    await this.customerService.getCustomerById(appId, billingCustomerId);

    // Verify payment method exists and belongs to customer
    const paymentMethod = await billingPrisma.billing_payment_methods.findFirst({
      where: {
        tilled_payment_method_id: tilledPaymentMethodId,
        billing_customer_id: billingCustomerId,
        app_id: appId,
        deleted_at: null
      }
    });

    if (!paymentMethod) {
      throw new NotFoundError(`Payment method ${tilledPaymentMethodId} not found for customer ${billingCustomerId}`);
    }

    // Detach from Tilled (best effort - continue if fails)
    try {
      const tilledClient = this.getTilledClient(appId);
      await tilledClient.detachPaymentMethod(tilledPaymentMethodId);
    } catch (error) {
      logger.warn('Failed to detach payment method from Tilled', {
        app_id: appId,
        billing_customer_id: billingCustomerId,
        tilled_payment_method_id: tilledPaymentMethodId,
        error_message: error.message
      });
    }

    // Soft delete locally using verified record id (avoid TOCTOU race)
    await billingPrisma.billing_payment_methods.update({
      where: { id: paymentMethod.id },
      data: {
        deleted_at: new Date(),
        is_default: false
      }
    });

    // If this was the default, clear customer fast-path
    if (paymentMethod.is_default) {
      await billingPrisma.billing_customers.update({
        where: { id: billingCustomerId },
        data: {
          default_payment_method_id: null,
          payment_method_type: null,
          updated_at: new Date()
        }
      });
    }

    return { deleted: true, deleted_at: new Date() };
  }
}

module.exports = PaymentMethodService;
