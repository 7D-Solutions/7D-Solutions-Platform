const { billingPrisma } = require('../../prisma');
const logger = require('@fireproof/infrastructure/utils/logger');
const DunningConfigService = require('../DunningConfigService');

/**
 * Webhook event handlers - Domain-specific logic for each webhook event type.
 *
 * Each handler operates on different database tables and represents
 * a separate business concern. Extracted from WebhookService for
 * separation of concerns.
 */

async function handleSubscriptionUpdate(appId, tilledSubscription) {
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

async function handleSubscriptionCanceled(appId, tilledSubscription) {
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

async function handlePaymentFailure(appId, paymentObject, eventType) {
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

  // Update customer delinquent status and grace period
  try {
    const dunningConfigService = new DunningConfigService();
    const config = await dunningConfigService.getConfig(appId);

    const now = new Date();
    const gracePeriodEnd = new Date(now);
    gracePeriodEnd.setDate(gracePeriodEnd.getDate() + config.gracePeriodDays);

    await billingPrisma.billing_customers.update({
      where: { id: subscription.billing_customer_id },
      data: {
        delinquent_since: now,
        grace_period_end: gracePeriodEnd,
        next_retry_at: null,
        retry_attempt_count: 0,
        status: 'delinquent'
      }
    });

    logger.info('Customer marked as delinquent with grace period', {
      app_id: appId,
      customer_id: subscription.billing_customer_id,
      grace_period_days: config.gracePeriodDays,
      grace_period_end: gracePeriodEnd
    });
  } catch (error) {
    logger.error('Failed to update customer delinquent status', {
      app_id: appId,
      billing_subscription_id: subscription.id,
      error: error.message
    });
  }

  // Note: Status will be updated via subscription.updated webhook
  // We log here for operational awareness but don't update status directly
}

async function handleRefundEvent(appId, tilledRefund) {
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

async function handleDisputeEvent(appId, tilledDispute) {
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

module.exports = {
  handleSubscriptionUpdate,
  handleSubscriptionCanceled,
  handlePaymentFailure,
  handleRefundEvent,
  handleDisputeEvent
};
