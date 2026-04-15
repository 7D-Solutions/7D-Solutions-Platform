use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::lifecycle;

/// Payload shape for ar.invoice_suspended events (matches AR's InvoiceSuspendedPayload)
#[derive(Debug, Clone, Deserialize)]
pub struct InvoiceSuspendedEvent {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub dunning_attempt: i32,
    pub reason: String,
}

/// Handle an ar.invoice_suspended event by suspending matching subscriptions.
///
/// **Cross-module contract**: AR emits `ar.invoice_suspended` when dunning
/// reaches terminal escalation. Subscriptions consumes this event and applies
/// suspension to the appropriate subscription(s) for that customer/tenant.
///
/// **Idempotency**: Uses the `processed_events` table — duplicate event_ids
/// are silently skipped.
///
/// **No cross-module DB writes**: Subscriptions only writes to its own DB.
pub async fn handle_invoice_suspended(
    pool: &PgPool,
    event_id: &str,
    event: &InvoiceSuspendedEvent,
) -> Result<bool, Box<dyn std::error::Error>> {
    process_event_idempotent(pool, event_id, "ar.invoice_suspended", || {
        let pool = pool.clone();
        let tenant_id = event.tenant_id.clone();
        let customer_id = event.customer_id.clone();
        let reason = event.reason.clone();
        async move {
            // Find active or past_due subscriptions for this customer in this tenant
            let subscription_ids: Vec<Uuid> = sqlx::query_scalar(
                r#"
                SELECT id FROM subscriptions
                WHERE tenant_id = $1
                  AND ar_customer_id = $2
                  AND status IN ('active', 'past_due')
                "#,
            )
            .bind(&tenant_id)
            .bind(&customer_id)
            .fetch_all(&pool)
            .await
            .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?;

            if subscription_ids.is_empty() {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    customer_id = %customer_id,
                    "No active/past_due subscriptions found for suspended invoice"
                );
                return Ok(());
            }

            for sub_id in subscription_ids {
                let suspend_reason = format!("dunning_suspension: {}", reason);
                match lifecycle::transition_to_suspended(sub_id, &tenant_id, &suspend_reason, &pool)
                    .await
                {
                    Ok(()) => {
                        tracing::info!(
                            subscription_id = %sub_id,
                            tenant_id = %tenant_id,
                            "Subscription suspended due to dunning escalation"
                        );
                    }
                    Err(lifecycle::TransitionError::IllegalTransition { .. }) => {
                        // Already suspended or in a state that can't transition — idempotent
                        tracing::debug!(
                            subscription_id = %sub_id,
                            "Subscription already in non-suspendable state, skipping"
                        );
                    }
                    Err(e) => {
                        return Err(Box::new(e) as Box<dyn std::error::Error>);
                    }
                }
            }

            Ok(())
        }
    })
    .await
}

/// Check if an event has already been processed (idempotency check)
pub async fn is_event_processed(pool: &PgPool, event_id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        SELECT event_id
        FROM processed_events
        WHERE event_id = $1
        "#,
        event_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(result.is_some())
}

/// Mark an event as processed to prevent duplicate processing
pub async fn mark_event_processed(
    pool: &PgPool,
    event_id: &str,
    subject: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO processed_events (event_id, subject)
        VALUES ($1, $2)
        ON CONFLICT (event_id) DO NOTHING
        "#,
        event_id,
        subject
    )
    .execute(pool)
    .await?;

    tracing::debug!("Marked event {} as processed", event_id);

    Ok(())
}

/// Process an event with idempotency guarantee
///
/// This function checks if the event has already been processed, and if not,
/// calls the provided handler function and then marks the event as processed.
pub async fn process_event_idempotent<F, Fut>(
    pool: &PgPool,
    event_id: &str,
    subject: &str,
    handler: F,
) -> Result<bool, Box<dyn std::error::Error>>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
    // Check if already processed
    if is_event_processed(pool, event_id).await? {
        tracing::debug!("Event {} already processed, skipping", event_id);
        return Ok(false);
    }

    // Process the event
    handler().await?;

    // Mark as processed
    mark_event_processed(pool, event_id, subject).await?;

    Ok(true)
}
