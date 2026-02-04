const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError, ConflictError, UnauthorizedError } = require('../utils/errors');

class WebhookService {
  constructor(getTilledClientFn) {
    this.getTilledClient = getTilledClientFn;
  }

  async processWebhook(appId, event, rawBody, signature) {
    // Step 1: Check idempotency FIRST (insert webhook record via unique event_id)
    // This prevents wasting CPU on signature verification for duplicates
    try {
      await billingPrisma.billing_webhooks.create({
        data: {
          app_id: appId,
          event_id: event.id,
          event_type: event.type,
          status: 'received'
        }
      });
    } catch (error) {
      // Unique violation = already processed
      if (error.code === 'P2002') {
        logger.info('Webhook already processed', { app_id: appId, event_id: event.id });
        return { success: true, duplicate: true };
      }
      throw error;
    }

    // Step 2: Verify signature (only for non-duplicates)
    const tilledClient = this.getTilledClient(appId);
    const isValid = tilledClient.verifyWebhookSignature(rawBody, signature);
    if (!isValid) {
      logger.warn('Invalid webhook signature', { app_id: appId, event_id: event.id });

      // Mark webhook as failed due to invalid signature
      await billingPrisma.billing_webhooks.update({
        where: {
          event_id_app_id: {
            event_id: event.id,
            app_id: appId
          }
        },
        data: { status: 'failed', error: 'Invalid signature', processed_at: new Date() }
      });

      return { success: false, error: 'Invalid signature' };
    }

    // Step 3: Process event
    try {
      await this.handleWebhookEvent(appId, event);

      await billingPrisma.billing_webhooks.update({
        where: {
          event_id_app_id: {
            event_id: event.id,
            app_id: appId
          }
        },
        data: { status: 'processed', processed_at: new Date() }
      });

      return { success: true, duplicate: false };
    } catch (error) {
      logger.error('Webhook processing error', { app_id: appId, event_id: event.id, error: error.message });

      await billingPrisma.billing_webhooks.update({
        where: {
          event_id_app_id: {
            event_id: event.id,
            app_id: appId
          }
        },
        data: { status: 'failed', error: error.message, processed_at: new Date() }
      });

      throw error;
    }
  }

  async handleWebhookEvent(appId, event) {
    switch (event.type) {
      case 'subscription.updated':
      case 'subscription.created':
        await this.handleSubscriptionUpdate(appId, event.data.object);
        break;
      case 'subscription.canceled':
      case 'subscription.deleted':
        await this.handleSubscriptionCanceled(appId, event.data.object);
        break;

      // Payment failure events - critical for status accuracy
      case 'charge.failed':
      case 'invoice.payment_failed':
      case 'payment_intent.payment_failed':
        await this.handlePaymentFailure(appId, event.data.object, event.type);
        break;

      // Refund events
      case 'refund.created':
      case 'refund.updated':
        await this.handleRefundEvent(appId, event.data.object);
        break;

      // Dispute events
      case 'dispute.created':
      case 'dispute.updated':
        await this.handleDisputeEvent(appId, event.data.object);
        break;

      default:
        logger.info('Unhandled webhook event', { app_id: appId, event_type: event.type });
    }
  }

  async handlePaymentFailure(appId, paymentObject, eventType) {
    // Extract subscription ID from payment object
    const subscriptionId = paymentObject.subscription_id || paymentObject.subscription;

    if (!subscriptionId) {
      logger.warn('Payment failure event without subscription_id', {
        event_type: eventType,
        payment_object_id: paymentObject.id
      });
      return;
    }

    const subscription = await billingPrisma.billing_subscriptions.findFirst({
      where: {
        tilled_subscription_id: subscriptionId,
        app_id: appId
      }
    });

    if (!subscription) {
      logger.warn('Subscription not found for payment failure', {
        tilled_subscription_id: subscriptionId,
        event_type: eventType
      });
      return;
    }

    // Log payment failure for operational visibility
    logger.error('Payment failure detected', {
      billing_subscription_id: subscription.id,
      tilled_subscription_id: subscriptionId,
      event_type: eventType,
      payment_id: paymentObject.id,
      failure_code: paymentObject.failure_code,
      failure_message: paymentObject.failure_message
    });

    // Note: Status will be updated via subscription.updated webhook
    // We log here for operational awareness but don't update status directly
  }

  async handleSubscriptionUpdate(appId, tilledSubscription) {
    const subscription = await billingPrisma.billing_subscriptions.findFirst({
      where: {
        tilled_subscription_id: tilledSubscription.id,
        app_id: appId
      }
    });

    if (!subscription) {
      logger.warn('Subscription not found', {
        app_id: appId,
        tilled_subscription_id: tilledSubscription.id
      });
      return;
    }

    await billingPrisma.billing_subscriptions.update({
      where: { id: subscription.id },
      data: {
        status: tilledSubscription.status,
        current_period_start: new Date(tilledSubscription.current_period_start * 1000),
        current_period_end: new Date(tilledSubscription.current_period_end * 1000),
        cancel_at: tilledSubscription.cancel_at ? new Date(tilledSubscription.cancel_at * 1000) : null,
        canceled_at: tilledSubscription.canceled_at ? new Date(tilledSubscription.canceled_at * 1000) : null,
        updated_at: new Date()
      }
    });
  }

  async handleSubscriptionCanceled(appId, tilledSubscription) {
    const subscription = await billingPrisma.billing_subscriptions.findFirst({
      where: {
        tilled_subscription_id: tilledSubscription.id,
        app_id: appId
      }
    });

    if (!subscription) {
      logger.warn('Subscription not found', {
        app_id: appId,
        tilled_subscription_id: tilledSubscription.id
      });
      return;
    }

    await billingPrisma.billing_subscriptions.update({
      where: { id: subscription.id },
      data: {
        status: 'canceled',
        canceled_at: new Date(),
        updated_at: new Date()
      }
    });
  }

  async handleRefundEvent(appId, tilledRefund) {
    // Extract charge/payment_intent reference
    const tilledChargeId = tilledRefund.payment_intent_id || tilledRefund.charge_id;

    if (!tilledChargeId) {
      logger.warn('Refund event missing payment_intent_id/charge_id', {
        app_id: appId,
        tilled_refund_id: tilledRefund.id
      });
      // Continue processing - we'll upsert by refund ID alone
    }

    // Try to find local charge for linkage
    let chargeId = null;
    let billingCustomerId = null;

    if (tilledChargeId) {
      const charge = await billingPrisma.billing_charges.findFirst({
        where: {
          app_id: appId,
          tilled_charge_id: tilledChargeId
        }
      });

      if (charge) {
        chargeId = charge.id;
        billingCustomerId = charge.billing_customer_id;
      } else {
        logger.warn('Charge not found for refund', {
          app_id: appId,
          tilled_charge_id: tilledChargeId,
          tilled_refund_id: tilledRefund.id
        });
      }
    }

    // Check if we already have this refund (for updates)
    const existingRefund = await billingPrisma.billing_refunds.findFirst({
      where: {
        tilled_refund_id: tilledRefund.id,
        app_id: appId
      }
    });

    if (existingRefund) {
      // Update existing refund
      await billingPrisma.billing_refunds.update({
        where: {
          tilled_refund_id_app_id: {
            tilled_refund_id: tilledRefund.id,
            app_id: appId
          }
        },
        data: {
          status: tilledRefund.status,
          failure_code: tilledRefund.failure_code || null,
          failure_message: tilledRefund.failure_message || null,
          updated_at: new Date()
        }
      });

      logger.info('Refund webhook updated', {
        app_id: appId,
        tilled_refund_id: tilledRefund.id,
        status: tilledRefund.status,
        refund_id: existingRefund.id
      });
    } else {
      // Create new refund only if we have the required linkages
      if (!chargeId || !billingCustomerId) {
        logger.warn('Cannot create refund from webhook: missing charge linkage', {
          app_id: appId,
          tilled_refund_id: tilledRefund.id,
          tilled_charge_id: tilledChargeId
        });
        return;
      }

      await billingPrisma.billing_refunds.create({
        data: {
          app_id: appId,
          tilled_refund_id: tilledRefund.id,
          tilled_charge_id: tilledChargeId,
          charge_id: chargeId,
          billing_customer_id: billingCustomerId,
          status: tilledRefund.status,
          amount_cents: tilledRefund.amount || 0,
          currency: tilledRefund.currency || 'usd',
          reason: tilledRefund.reason,
          reference_id: `webhook:${tilledRefund.id}`, // Webhook-generated reference_id
          failure_code: tilledRefund.failure_code || null,
          failure_message: tilledRefund.failure_message || null
        }
      });

      logger.info('Refund webhook created', {
        app_id: appId,
        tilled_refund_id: tilledRefund.id,
        status: tilledRefund.status,
        charge_id: chargeId
      });
    }
  }

  async handleDisputeEvent(appId, tilledDispute) {
    // Extract charge/payment_intent reference
    const tilledChargeId = tilledDispute.payment_intent_id || tilledDispute.charge_id;

    // Try to find local charge for linkage
    let chargeId = null;

    if (tilledChargeId) {
      const charge = await billingPrisma.billing_charges.findFirst({
        where: {
          app_id: appId,
          tilled_charge_id: tilledChargeId
        }
      });

      if (charge) {
        chargeId = charge.id;
      } else {
        logger.warn('Charge not found for dispute', {
          app_id: appId,
          tilled_charge_id: tilledChargeId,
          tilled_dispute_id: tilledDispute.id
        });
      }
    }

    // Upsert dispute by tilled_dispute_id + app_id
    try {
      await billingPrisma.billing_disputes.upsert({
        where: {
          tilled_dispute_id_app_id: {
            tilled_dispute_id: tilledDispute.id,
            app_id: appId
          }
        },
        update: {
          status: tilledDispute.status,
          ...(chargeId && { charge_id: chargeId }),
          amount_cents: tilledDispute.amount || null,
          currency: tilledDispute.currency || null,
          reason: tilledDispute.reason || null,
          reason_code: tilledDispute.reason_code || null,
          evidence_due_by: tilledDispute.evidence_due_by ? new Date(tilledDispute.evidence_due_by * 1000) : null,
          closed_at: tilledDispute.closed_at ? new Date(tilledDispute.closed_at * 1000) : null,
          updated_at: new Date()
        },
        create: {
          app_id: appId,
          tilled_dispute_id: tilledDispute.id,
          tilled_charge_id: tilledChargeId,
          charge_id: chargeId,
          status: tilledDispute.status,
          amount_cents: tilledDispute.amount || null,
          currency: tilledDispute.currency || null,
          reason: tilledDispute.reason || null,
          reason_code: tilledDispute.reason_code || null,
          evidence_due_by: tilledDispute.evidence_due_by ? new Date(tilledDispute.evidence_due_by * 1000) : null,
          opened_at: tilledDispute.opened_at ? new Date(tilledDispute.opened_at * 1000) : new Date(),
          closed_at: tilledDispute.closed_at ? new Date(tilledDispute.closed_at * 1000) : null,
          created_at: tilledDispute.created ? new Date(tilledDispute.created * 1000) : new Date()
        }
      });

      logger.info('Dispute webhook processed', {
        app_id: appId,
        tilled_dispute_id: tilledDispute.id,
        status: tilledDispute.status,
        charge_id: chargeId
      });
    } catch (error) {
      logger.error('Dispute webhook upsert failed', {
        app_id: appId,
        tilled_dispute_id: tilledDispute.id,
        error: error.message
      });
      throw error;
    }
  }
}

module.exports = WebhookService;
