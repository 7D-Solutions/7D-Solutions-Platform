use sqlx::PgPool;

use crate::domain::payment_methods;
use crate::models::TilledWebhookEvent;

/// Process payment method webhook events.
///
/// `attached`: Bind provider ID to a `pending_sync` PM or update existing.
/// Hydrate card details (brand, last4, exp). `detached`: Idempotent soft-delete.
pub(super) async fn process_payment_method_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let pm_data = &event.data;
    let tilled_pm_id = pm_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing payment method ID".to_string())?;

    if event.event_type == "payment_method.attached" {
        process_attached(db, app_id, event, pm_data, tilled_pm_id).await?;
    } else {
        // payment_method.detached — idempotent soft-delete.
        payment_methods::webhook_detach(db, tilled_pm_id, app_id)
            .await
            .map_err(|e| format!("Failed to detach payment method: {}", e))?;
    }

    tracing::info!("Processed payment method event for {}", tilled_pm_id);
    Ok(())
}

async fn process_attached(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
    pm_data: &serde_json::Value,
    tilled_pm_id: &str,
) -> Result<(), String> {
    let card = pm_data.get("card");
    let brand = card.and_then(|c| c.get("brand")).and_then(|v| v.as_str());
    let last4 = card.and_then(|c| c.get("last4")).and_then(|v| v.as_str());
    let exp_month = card
        .and_then(|c| c.get("exp_month"))
        .and_then(|v| v.as_i64());
    let exp_year = card
        .and_then(|c| c.get("exp_year"))
        .and_then(|v| v.as_i64());
    let pm_type = pm_data
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("card");

    let tilled_customer_id = pm_data.get("customer").and_then(|v| v.as_str());

    let mut bound = false;
    if let Some(cust_id) = tilled_customer_id {
        let rows = payment_methods::bind_pending_by_tilled_id(
            db,
            tilled_pm_id,
            brand,
            last4,
            exp_month.map(|v| v as i32),
            exp_year.map(|v| v as i32),
            pm_type,
            &event.data,
            app_id,
        )
        .await
        .map_err(|e| format!("Failed to bind pending PM: {}", e))?;
        bound = rows > 0;

        if !bound {
            let rows2 = payment_methods::bind_pending_by_customer(
                db,
                tilled_pm_id,
                brand,
                last4,
                exp_month.map(|v| v as i32),
                exp_year.map(|v| v as i32),
                pm_type,
                &event.data,
                app_id,
                cust_id,
            )
            .await
            .map_err(|e| format!("Failed to bind pending PM by customer: {}", e))?;
            bound = rows2 > 0;
        }
    }

    if !bound {
        payment_methods::update_active_details(
            db,
            brand,
            last4,
            exp_month.map(|v| v as i32),
            exp_year.map(|v| v as i32),
            pm_type,
            &event.data,
            tilled_pm_id,
            app_id,
        )
        .await
        .map_err(|e| format!("Failed to update payment method: {}", e))?;
    }

    Ok(())
}
