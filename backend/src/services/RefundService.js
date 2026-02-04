const logger = require('@fireproof/infrastructure/utils/logger');
const { getBillingPrisma } = require('../prisma.factory');
const { NotFoundError, ValidationError, ConflictError, UnauthorizedError } = require('../utils/errors');

class RefundService {
  constructor(getTilledClientFn) {
    this.getTilledClient = getTilledClientFn;
  }

  async createRefund(
    appId,
    {
      chargeId,
      amountCents,
      currency = 'usd',
      reason,
      referenceId,
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
    if (!chargeId) {
      throw new ValidationError('chargeId is required');
    }
    // Enforce reference_id as strictly required (domain-level idempotency key)
    if (!referenceId || referenceId.trim() === '') {
      throw new ValidationError('referenceId is required');
    }

    // Domain idempotency check FIRST (before loading charge)
    // This short-circuits if we already processed this reference_id
    const existingRefund = await getBillingPrisma().billing_refunds.findFirst({
      where: {
        app_id: appId,
        reference_id: referenceId,
      },
    });

    if (existingRefund) {
      logger.info('Returning existing refund for duplicate reference_id', {
        app_id: appId,
        reference_id: referenceId,
        refund_id: existingRefund.id,
      });
      return existingRefund;
    }

    // Load charge with app_id scoping and customer linkage
    const charge = await getBillingPrisma().billing_charges.findFirst({
      where: {
        id: chargeId,
        app_id: appId, // CRITICAL: app_id scoping prevents cross-app access
      },
      include: {
        customer: true,
      },
    });

    if (!charge) {
      // Return 404 whether charge doesn't exist or belongs to different app (no ID leakage)
      throw new NotFoundError('Charge not found');
    }

    // Ensure charge has been settled in processor
    if (!charge.tilled_charge_id) {
      throw new ConflictError('Charge not settled in processor');
    }

    // Create pending refund record (with race-safe duplicate detection)
    let refundRecord;
    try {
      refundRecord = await getBillingPrisma().billing_refunds.create({
        data: {
          app_id: appId,
          billing_customer_id: charge.billing_customer_id,
          charge_id: charge.id,
          tilled_charge_id: charge.tilled_charge_id,
          status: 'pending',
          amount_cents: amountCents,
          currency,
          reason,
          reference_id: referenceId,
          note,
          metadata,
          tilled_refund_id: null,
        },
      });
    } catch (error) {
      // Handle race condition: two concurrent requests with same reference_id
      if (error.code === 'P2002' && error.meta?.target?.includes('unique_refund_app_reference_id')) {
        logger.info('Race condition detected: duplicate reference_id on create, fetching existing', {
          app_id: appId,
          reference_id: referenceId,
        });

        const existingRefundRace = await getBillingPrisma().billing_refunds.findFirst({
          where: {
            app_id: appId,
            reference_id: referenceId,
          },
        });

        if (existingRefundRace) {
          return existingRefundRace;
        }
      }

      // Re-throw if not a reference_id duplicate
      throw error;
    }

    // Call Tilled to create refund
    const tilledClient = this.getTilledClient(appId);

    try {
      const tilledRefund = await tilledClient.createRefund({
        appId,
        tilledChargeId: charge.tilled_charge_id,
        amountCents,
        currency,
        reason,
        metadata: {
          reference_id: referenceId,
          ...metadata,
        },
      });

      // Update refund record with success
      const updatedRefund = await getBillingPrisma().billing_refunds.update({
        where: { id: refundRecord.id },
        data: {
          status: tilledRefund.status || 'succeeded',
          tilled_refund_id: tilledRefund.id,
        },
      });

      logger.info('Refund succeeded', {
        app_id: appId,
        refund_id: updatedRefund.id,
        tilled_refund_id: tilledRefund.id,
        charge_id: charge.id,
        tilled_charge_id: charge.tilled_charge_id,
        amount_cents: amountCents,
        reason,
      });

      return updatedRefund;
    } catch (error) {
      // Update refund record with failure
      await getBillingPrisma().billing_refunds.update({
        where: { id: refundRecord.id },
        data: {
          status: 'failed',
          failure_code: error.code || 'unknown',
          failure_message: error.message,
        },
      });

      logger.error('Refund failed', {
        app_id: appId,
        refund_id: refundRecord.id,
        charge_id: charge.id,
        tilled_charge_id: charge.tilled_charge_id,
        error_code: error.code,
        error_message: error.message,
        amount_cents: amountCents,
        reason,
      });

      throw error;
    }
  }

  async getRefund(appId, refundId) {
    const refund = await getBillingPrisma().billing_refunds.findFirst({
      where: {
        id: refundId,
        app_id: appId, // CRITICAL: app_id scoping
      },
      include: {
        charge: true,
        customer: true,
      },
    });

    if (!refund) {
      throw new NotFoundError('Refund not found');
    }

    return refund;
  }

  async listRefunds(appId, { chargeId, status, limit = 100, offset = 0 } = {}) {
    const where = {
      app_id: appId, // CRITICAL: app_id scoping
      ...(chargeId && { charge_id: chargeId }),
      ...(status && { status }),
    };

    const refunds = await getBillingPrisma().billing_refunds.findMany({
      where,
      include: {
        charge: {
          select: {
            id: true,
            tilled_charge_id: true,
            amount_cents: true,
            reason: true,
          },
        },
      },
      orderBy: {
        created_at: 'desc',
      },
      take: limit,
      skip: offset,
    });

    return refunds;
  }
}

module.exports = RefundService;
