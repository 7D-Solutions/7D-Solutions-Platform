use sqlx::PgPool;

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
        sqlx::query(
            r#"
            UPDATE ar_payment_methods
            SET status = 'inactive', deleted_at = COALESCE(deleted_at, NOW()),
                updated_at = NOW()
            WHERE tilled_payment_method_id = $1 AND app_id = $2
              AND status != 'inactive'
            "#,
        )
        .bind(tilled_pm_id)
        .bind(app_id)
        .execute(db)
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

    // Try to bind a pending_sync PM by customer.
    let tilled_customer_id = pm_data.get("customer").and_then(|v| v.as_str());

    let mut bound = false;
    if let Some(cust_id) = tilled_customer_id {
        let res = sqlx::query(
            r#"
            UPDATE ar_payment_methods
            SET tilled_payment_method_id = $1, status = 'active',
                brand = COALESCE($2, brand), last4 = COALESCE($3, last4),
                exp_month = COALESCE($4, exp_month), exp_year = COALESCE($5, exp_year),
                type = $6, metadata = $7, updated_at = NOW()
            WHERE app_id = $8 AND status = 'pending_sync'
              AND tilled_payment_method_id = $1
            "#,
        )
        .bind(tilled_pm_id)
        .bind(brand)
        .bind(last4)
        .bind(exp_month.map(|v| v as i32))
        .bind(exp_year.map(|v| v as i32))
        .bind(pm_type)
        .bind(&event.data)
        .bind(app_id)
        .execute(db)
        .await
        .map_err(|e| format!("Failed to bind pending PM: {}", e))?;
        bound = res.rows_affected() > 0;

        // If no pending_sync with that ID, try matching by customer with pending_sync.
        if !bound {
            let res2 = sqlx::query(
                r#"
                UPDATE ar_payment_methods
                SET tilled_payment_method_id = $1, status = 'active',
                    brand = COALESCE($2, brand), last4 = COALESCE($3, last4),
                    exp_month = COALESCE($4, exp_month), exp_year = COALESCE($5, exp_year),
                    type = $6, metadata = $7, updated_at = NOW()
                WHERE app_id = $8 AND status = 'pending_sync'
                  AND ar_customer_id = (
                      SELECT id FROM ar_customers
                      WHERE tilled_customer_id = $9 AND app_id = $8 LIMIT 1
                  )
                "#,
            )
            .bind(tilled_pm_id)
            .bind(brand)
            .bind(last4)
            .bind(exp_month.map(|v| v as i32))
            .bind(exp_year.map(|v| v as i32))
            .bind(pm_type)
            .bind(&event.data)
            .bind(app_id)
            .bind(cust_id)
            .execute(db)
            .await
            .map_err(|e| format!("Failed to bind pending PM by customer: {}", e))?;
            bound = res2.rows_affected() > 0;
        }
    }

    if !bound {
        // Update existing active PM with latest card details.
        sqlx::query(
            r#"
            UPDATE ar_payment_methods
            SET brand = COALESCE($1, brand), last4 = COALESCE($2, last4),
                exp_month = COALESCE($3, exp_month), exp_year = COALESCE($4, exp_year),
                type = $5, status = 'active', metadata = $6,
                deleted_at = NULL, updated_at = NOW()
            WHERE tilled_payment_method_id = $7 AND app_id = $8
            "#,
        )
        .bind(brand)
        .bind(last4)
        .bind(exp_month.map(|v| v as i32))
        .bind(exp_year.map(|v| v as i32))
        .bind(pm_type)
        .bind(&event.data)
        .bind(tilled_pm_id)
        .bind(app_id)
        .execute(db)
        .await
        .map_err(|e| format!("Failed to update payment method: {}", e))?;
    }

    Ok(())
}
