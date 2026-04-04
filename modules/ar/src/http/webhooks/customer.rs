use sqlx::PgPool;

use crate::domain::customers;
use crate::models::TilledWebhookEvent;

/// Process customer webhook events.
///
/// Idempotent upsert by `tilled_customer_id`. If a local customer exists in
/// `pending_sync` with a matching email, bind the provider ID and activate.
pub(super) async fn process_customer_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let customer_data = &event.data;
    let tilled_customer_id = customer_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing customer ID in webhook data".to_string())?;

    let email = customer_data
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let name = customer_data.get("name").and_then(|v| v.as_str());
    let webhook_status = customer_data
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("active");

    // First: try to bind a pending_sync customer by email (provider ID not yet set).
    let bound = customers::bind_pending_by_email(
        db,
        tilled_customer_id,
        name,
        &event.data,
        app_id,
        email,
    )
    .await
    .map_err(|e| format!("Failed to bind pending customer: {}", e))?;

    if bound > 0 {
        tracing::info!(
            "Bound pending_sync customer to tilled_customer_id={}",
            tilled_customer_id
        );
        return Ok(());
    }

    // Upsert by tilled_customer_id (globally unique).
    customers::upsert_by_tilled_id(
        db,
        app_id,
        tilled_customer_id,
        email,
        name,
        webhook_status,
        &event.data,
    )
    .await
    .map_err(|e| format!("Failed to upsert customer: {}", e))?;

    tracing::info!("Processed customer event for {}", tilled_customer_id);
    Ok(())
}
