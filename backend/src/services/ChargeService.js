const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError, ConflictError, UnauthorizedError } = require('../utils/errors');

class ChargeService {
  constructor(getTilledClientFn) {
    this.getTilledClient = getTilledClientFn;
  }

  async createOneTimeCharge(
    appId,
    {
      externalCustomerId,
      amountCents,
      currency = 'usd',
      reason,
      referenceId,
      serviceDate,
      note,
      metadata,
    },
    { idempotencyKey, requestHash }
  ) {
    // Validate required fields
    if (amountCents === undefined || amountCents === null) {
      throw new ValidationError('amountCents is required');
    }
    if (amountCents <= 0) {
      throw new ValidationError('amountCents must be greater than 0');
    }
    if (!reason) {
      throw new ValidationError('reason is required');
    }
    // Enforce reference_id as strictly required (domain-level idempotency key)
    if (!referenceId || referenceId.trim() === '') {
      throw new ValidationError('referenceId is required');
    }

    // Lookup billing customer
    const customer = await billingPrisma.billing_customers.findFirst({
      where: {
        app_id: appId,
        external_customer_id: String(externalCustomerId),
      },
    });

    if (!customer) {
      throw new NotFoundError('Customer not found');
    }

    // Ensure default payment method exists
    if (!customer.default_payment_method_id) {
      throw new ConflictError('No default payment method on file');
    }

    // Check for duplicate reference_id (idempotent by reference_id)
    const existingCharge = await billingPrisma.billing_charges.findFirst({
      where: {
        app_id: appId,
        reference_id: referenceId,
      },
    });

    if (existingCharge) {
      logger.info('Returning existing charge for duplicate reference_id', {
        app_id: appId,
        reference_id: referenceId,
        charge_id: existingCharge.id,
      });
      return existingCharge;
    }

    // Create pending charge record (with race-safe duplicate detection)
    let chargeRecord;
    try {
      chargeRecord = await billingPrisma.billing_charges.create({
        data: {
          app_id: appId,
          billing_customer_id: customer.id,
          subscription_id: null,
          invoice_id: null,
          status: 'pending',
          amount_cents: amountCents,
          currency,
          charge_type: 'one_time', // Future-proof: distinguishes from invoice/subscription charges
          reason,
          reference_id: referenceId,
          service_date: serviceDate ? new Date(serviceDate) : null,
          note,
          metadata,
          tilled_charge_id: null,
        },
      });
    } catch (error) {
      // Handle race condition: two concurrent requests with same reference_id
      if (error.code === 'P2002' && error.meta?.target?.includes('unique_app_reference_id')) {
        logger.info('Race condition detected: duplicate reference_id on create, fetching existing', {
          app_id: appId,
          reference_id: referenceId,
        });

        const existingChargeRace = await billingPrisma.billing_charges.findFirst({
          where: {
            app_id: appId,
            reference_id: referenceId,
          },
        });

        if (existingChargeRace) {
          return existingChargeRace;
        }
      }

      // Re-throw if not a reference_id duplicate
      throw error;
    }

    // Call Tilled to create charge
    const tilledClient = this.getTilledClient(appId);

    try {
      const tilledCharge = await tilledClient.createCharge({
        appId,
        tilledCustomerId: customer.tilled_customer_id,
        paymentMethodId: customer.default_payment_method_id,
        amountCents,
        currency,
        description: reason,
        metadata: {
          reference_id: referenceId,
          service_date: serviceDate,
          ...metadata,
        },
      });

      // Update charge record with success
      const updatedCharge = await billingPrisma.billing_charges.update({
        where: { id: chargeRecord.id },
        data: {
          status: tilledCharge.status || 'succeeded',
          tilled_charge_id: tilledCharge.id,
        },
      });

      logger.info('One-time charge succeeded', {
        app_id: appId,
        charge_id: updatedCharge.id,
        tilled_charge_id: tilledCharge.id,
        amount_cents: amountCents,
        reason,
      });

      return updatedCharge;
    } catch (error) {
      // Update charge record with failure
      await billingPrisma.billing_charges.update({
        where: { id: chargeRecord.id },
        data: {
          status: 'failed',
          failure_code: error.code || 'unknown',
          failure_message: error.message,
        },
      });

      logger.error('One-time charge failed', {
        app_id: appId,
        charge_id: chargeRecord.id,
        error_code: error.code,
        error_message: error.message,
        amount_cents: amountCents,
        reason,
      });

      throw error;
    }
  }
}

module.exports = ChargeService;
