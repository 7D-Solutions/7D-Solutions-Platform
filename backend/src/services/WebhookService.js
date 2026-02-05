const { billingPrisma } = require('../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const { NotFoundError, ValidationError, ConflictError, UnauthorizedError } = require('../utils/errors');
const handlers = require('./helpers/webhookHandlers');

/**
 * WebhookService - Webhook lifecycle management
 *
 * Handles idempotency (via event_id), signature verification, and status tracking.
 * Event-specific domain logic is delegated to helpers/webhookHandlers.js.
 */

const HANDLER_MAP = {
  'subscription.updated': (appId, event) => handlers.handleSubscriptionUpdate(appId, event.data.object),
  'subscription.created': (appId, event) => handlers.handleSubscriptionUpdate(appId, event.data.object),
  'subscription.canceled': (appId, event) => handlers.handleSubscriptionCanceled(appId, event.data.object),
  'subscription.deleted': (appId, event) => handlers.handleSubscriptionCanceled(appId, event.data.object),
  'charge.failed': (appId, event) => handlers.handlePaymentFailure(appId, event.data.object, event.type),
  'invoice.payment_failed': (appId, event) => handlers.handlePaymentFailure(appId, event.data.object, event.type),
  'payment_intent.payment_failed': (appId, event) => handlers.handlePaymentFailure(appId, event.data.object, event.type),
  'refund.created': (appId, event) => handlers.handleRefundEvent(appId, event.data.object),
  'refund.updated': (appId, event) => handlers.handleRefundEvent(appId, event.data.object),
  'dispute.created': (appId, event) => handlers.handleDisputeEvent(appId, event.data.object),
  'dispute.updated': (appId, event) => handlers.handleDisputeEvent(appId, event.data.object)
};

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
    const handler = HANDLER_MAP[event.type];
    if (handler) {
      await handler(appId, event);
    } else {
      logger.info('Unhandled webhook event', { app_id: appId, event_type: event.type });
    }
  }

  // Keep backward-compatible instance method delegates for BillingService facade
  async handlePaymentFailure(appId, paymentObject, eventType) {
    return handlers.handlePaymentFailure(appId, paymentObject, eventType);
  }

  async handleSubscriptionUpdate(appId, tilledSubscription) {
    return handlers.handleSubscriptionUpdate(appId, tilledSubscription);
  }

  async handleSubscriptionCanceled(appId, tilledSubscription) {
    return handlers.handleSubscriptionCanceled(appId, tilledSubscription);
  }

  async handleRefundEvent(appId, tilledRefund) {
    return handlers.handleRefundEvent(appId, tilledRefund);
  }

  async handleDisputeEvent(appId, tilledDispute) {
    return handlers.handleDisputeEvent(appId, tilledDispute);
  }
}

module.exports = WebhookService;
