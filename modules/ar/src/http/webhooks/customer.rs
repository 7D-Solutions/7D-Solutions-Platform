use sqlx::PgPool;

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
    let bound = sqlx::query(
        r#"
        UPDATE ar_customers
        SET tilled_customer_id = $1, status = 'active', name = COALESCE($2, name),
            metadata = $3, update_source = 'webhook', updated_at = NOW()
        WHERE app_id = $4 AND email = $5
          AND status = 'pending_sync' AND tilled_customer_id IS NULL
        "#,
    )
    .bind(tilled_customer_id)
    .bind(name)
    .bind(&event.data)
    .bind(app_id)
    .bind(email)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to bind pending customer: {}", e))?;

    if bound.rows_affected() > 0 {
        tracing::info!(
            "Bound pending_sync customer to tilled_customer_id={}",
            tilled_customer_id
        );
        return Ok(());
    }

    // Upsert by tilled_customer_id (globally unique).
    sqlx::query(
        r#"
        INSERT INTO ar_customers (
            app_id, tilled_customer_id, email, name, status, metadata,
            update_source, retry_attempt_count, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, 'webhook', 0, NOW(), NOW())
        ON CONFLICT (tilled_customer_id)
        DO UPDATE SET
            email = EXCLUDED.email,
            name = COALESCE(EXCLUDED.name, ar_customers.name),
            status = EXCLUDED.status,
            metadata = EXCLUDED.metadata,
            update_source = 'webhook',
            updated_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind(tilled_customer_id)
    .bind(email)
    .bind(name)
    .bind(webhook_status)
    .bind(&event.data)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to upsert customer: {}", e))?;

    tracing::info!("Processed customer event for {}", tilled_customer_id);
    Ok(())
}
