//! Webhook routing — translates `(system, source_event_type)` to a platform
//! domain event type and writes the routed event to the outbox atomically.

use chrono::Utc;
use uuid::Uuid;

use crate::events::{
    build_webhook_routed_envelope, WebhookRoutedPayload, EVENT_TYPE_WEBHOOK_ROUTED,
};
use crate::outbox::enqueue_event_tx;

use super::models::WebhookError;

/// Map a `(system, source_event_type)` pair to a platform domain event type.
///
/// Returns `None` when the source event is unknown or intentionally ignored.
pub fn map_to_domain_event(system: &str, source_event_type: Option<&str>) -> Option<String> {
    match (system, source_event_type) {
        // Stripe payment events
        ("stripe", Some("payment_intent.succeeded")) => Some("payment.received".to_string()),
        ("stripe", Some("payment_intent.payment_failed")) => Some("payment.failed".to_string()),
        ("stripe", Some("invoice.payment_succeeded")) => Some("invoice.paid.external".to_string()),
        ("stripe", Some("invoice.payment_failed")) => {
            Some("invoice.payment_failed.external".to_string())
        }
        ("stripe", Some("customer.subscription.created")) => {
            Some("subscription.created.external".to_string())
        }
        ("stripe", Some("customer.subscription.deleted")) => {
            Some("subscription.cancelled.external".to_string())
        }
        // GitHub events
        ("github", Some("push")) => Some("repository.push".to_string()),
        ("github", Some("pull_request")) => Some("repository.pull_request".to_string()),
        // QuickBooks Online (CloudEvents)
        ("quickbooks", Some("qbo.customer.created.v1")) => {
            Some("party.customer.synced".to_string())
        }
        ("quickbooks", Some("qbo.customer.updated.v1")) => {
            Some("party.customer.synced".to_string())
        }
        ("quickbooks", Some("qbo.invoice.created.v1")) => Some("ar.invoice.synced".to_string()),
        ("quickbooks", Some("qbo.invoice.updated.v1")) => Some("ar.invoice.synced".to_string()),
        ("quickbooks", Some("qbo.payment.created.v1")) => {
            Some("payments.payment.synced".to_string())
        }
        ("quickbooks", Some("qbo.item.updated.v1")) => Some("inventory.item.synced".to_string()),
        // Internal pass-through
        ("internal", Some(et)) => Some(et.to_string()),
        // Unknown — do not route
        _ => None,
    }
}

/// Emit a `webhook.routed` event to the outbox within an existing transaction.
///
/// This is called only when `map_to_domain_event` returns `Some(...)`.
pub async fn emit_routed_event_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ingest_id: i64,
    app_id: &str,
    system: &str,
    source_event_type: Option<&str>,
    domain_event_type: &str,
    correlation_id: &str,
) -> Result<Uuid, WebhookError> {
    let outbox_event_id = Uuid::new_v4();
    let envelope = build_webhook_routed_envelope(
        outbox_event_id,
        app_id.to_string(),
        correlation_id.to_string(),
        None,
        WebhookRoutedPayload {
            ingest_id,
            system: system.to_string(),
            source_event_type: source_event_type.map(str::to_string),
            domain_event_type: domain_event_type.to_string(),
            outbox_event_id,
            routed_at: Utc::now(),
        },
    );

    enqueue_event_tx(
        tx,
        outbox_event_id,
        EVENT_TYPE_WEBHOOK_ROUTED,
        "webhook",
        &ingest_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    Ok(outbox_event_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stripe_payment_intent_succeeded() {
        let result = map_to_domain_event("stripe", Some("payment_intent.succeeded"));
        assert_eq!(result, Some("payment.received".to_string()));
    }

    #[test]
    fn test_stripe_invoice_paid() {
        let result = map_to_domain_event("stripe", Some("invoice.payment_succeeded"));
        assert_eq!(result, Some("invoice.paid.external".to_string()));
    }

    #[test]
    fn test_unknown_stripe_event_not_routed() {
        let result = map_to_domain_event("stripe", Some("charge.dispute.created"));
        assert_eq!(result, None);
    }

    #[test]
    fn test_internal_passthrough() {
        let result = map_to_domain_event("internal", Some("my.custom.event"));
        assert_eq!(result, Some("my.custom.event".to_string()));
    }

    #[test]
    fn test_no_event_type() {
        let result = map_to_domain_event("stripe", None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_qbo_customer_created() {
        let result = map_to_domain_event("quickbooks", Some("qbo.customer.created.v1"));
        assert_eq!(result, Some("party.customer.synced".to_string()));
    }

    #[test]
    fn test_qbo_invoice_updated() {
        let result = map_to_domain_event("quickbooks", Some("qbo.invoice.updated.v1"));
        assert_eq!(result, Some("ar.invoice.synced".to_string()));
    }

    #[test]
    fn test_qbo_payment_created() {
        let result = map_to_domain_event("quickbooks", Some("qbo.payment.created.v1"));
        assert_eq!(result, Some("payments.payment.synced".to_string()));
    }

    #[test]
    fn test_qbo_item_updated() {
        let result = map_to_domain_event("quickbooks", Some("qbo.item.updated.v1"));
        assert_eq!(result, Some("inventory.item.synced".to_string()));
    }

    #[test]
    fn test_qbo_unknown_event_not_routed() {
        let result = map_to_domain_event("quickbooks", Some("qbo.unknown.v1"));
        assert_eq!(result, None);
    }
}
