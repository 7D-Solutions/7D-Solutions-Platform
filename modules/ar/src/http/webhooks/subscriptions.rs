use sqlx::PgPool;

use crate::domain::subscriptions;
use crate::models::TilledWebhookEvent;

/// Process subscription webhook events.
///
/// `created`: Bind `tilled_subscription_id`, set active, populate periods.
/// `updated`: Update status + metadata; out-of-order guard on terminal states.
/// `canceled`: Set canceled + `canceled_at`; terminal — cannot be regressed.
pub(super) async fn process_subscription_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let sub_data = &event.data;
    let tilled_sub_id = sub_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing subscription ID".to_string())?;

    let status = match event.event_type.as_str() {
        "subscription.created" => "active",
        "subscription.updated" => sub_data
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("active"),
        "subscription.canceled" => "canceled",
        _ => "active",
    };

    // Extract period fields if present.
    let current_period_start = parse_period_timestamp(sub_data, "current_period_start");
    let current_period_end = parse_period_timestamp(sub_data, "current_period_end");

    if event.event_type == "subscription.created" {
        handle_created(
            db,
            app_id,
            event,
            tilled_sub_id,
            current_period_start,
            current_period_end,
        )
        .await?;
    } else if event.event_type == "subscription.canceled" {
        handle_canceled(db, app_id, event, tilled_sub_id).await?;
    } else {
        handle_updated(
            db,
            app_id,
            event,
            tilled_sub_id,
            status,
            current_period_start,
            current_period_end,
        )
        .await?;
    }

    tracing::info!("Processed subscription event for {}", tilled_sub_id);
    Ok(())
}

fn parse_period_timestamp(data: &serde_json::Value, field: &str) -> Option<chrono::NaiveDateTime> {
    data.get(field).and_then(|v| v.as_str()).and_then(|s| {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ")
            .ok()
            .or_else(|| {
                s.parse::<i64>().ok().map(|ts| {
                    chrono::DateTime::from_timestamp(ts, 0)
                        .unwrap_or_default()
                        .naive_utc()
                })
            })
    })
}

async fn handle_created(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
    tilled_sub_id: &str,
    current_period_start: Option<chrono::NaiveDateTime>,
    current_period_end: Option<chrono::NaiveDateTime>,
) -> Result<(), String> {
    let tilled_customer_id = event.data.get("customer").and_then(|v| v.as_str());
    let mut bound = false;

    if let Some(cust_id) = tilled_customer_id {
        let rows = subscriptions::bind_pending_by_customer(
            db,
            tilled_sub_id,
            &event.data,
            current_period_start,
            current_period_end,
            app_id,
            cust_id,
        )
        .await
        .map_err(|e| format!("Failed to bind pending subscription: {}", e))?;
        bound = rows > 0;
    }

    if !bound {
        subscriptions::update_by_tilled_id_created(
            db,
            &event.data,
            current_period_start,
            current_period_end,
            tilled_sub_id,
            app_id,
        )
        .await
        .map_err(|e| format!("Failed to update subscription: {}", e))?;
    }

    Ok(())
}

async fn handle_canceled(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
    tilled_sub_id: &str,
) -> Result<(), String> {
    subscriptions::webhook_set_canceled(db, &event.data, tilled_sub_id, app_id)
        .await
        .map_err(|e| format!("Failed to cancel subscription: {}", e))?;
    Ok(())
}

async fn handle_updated(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
    tilled_sub_id: &str,
    status: &str,
    current_period_start: Option<chrono::NaiveDateTime>,
    current_period_end: Option<chrono::NaiveDateTime>,
) -> Result<(), String> {
    subscriptions::webhook_update(
        db,
        status,
        &event.data,
        current_period_start,
        current_period_end,
        tilled_sub_id,
        app_id,
    )
    .await
    .map_err(|e| format!("Failed to update subscription: {}", e))?;
    Ok(())
}
