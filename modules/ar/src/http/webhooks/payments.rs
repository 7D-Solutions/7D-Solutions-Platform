use sqlx::PgPool;

use crate::models::TilledWebhookEvent;

/// Process payment intent webhook events.
///
/// Out-of-order guard: `succeeded` and `failed` are terminal — older events
/// cannot regress them. If charge exists by `tilled_charge_id`, update it.
/// If a pending charge with NULL provider ID matches by customer, bind it.
pub(super) async fn process_payment_intent_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let pi_data = &event.data;
    let payment_intent_id = pi_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing payment intent ID".to_string())?;

    let status = if event.event_type == "payment_intent.succeeded" {
        "succeeded"
    } else {
        "failed"
    };

    let amount = pi_data.get("amount").and_then(|v| v.as_i64());
    let currency = pi_data
        .get("currency")
        .and_then(|v| v.as_str())
        .unwrap_or("usd");
    let failure_code = pi_data
        .get("last_payment_error")
        .and_then(|e| e.get("code"))
        .and_then(|v| v.as_str());
    let failure_message = pi_data
        .get("last_payment_error")
        .and_then(|e| e.get("message"))
        .and_then(|v| v.as_str());

    // Try to update existing charge by tilled_charge_id (out-of-order guard).
    let updated = sqlx::query(
        r#"
        UPDATE ar_charges
        SET status = $1, metadata = $2,
            amount_cents = COALESCE($3, amount_cents),
            currency = $4,
            failure_code = COALESCE($5, failure_code),
            failure_message = COALESCE($6, failure_message),
            updated_at = NOW()
        WHERE tilled_charge_id = $7 AND app_id = $8
          AND status NOT IN ('succeeded', 'failed', 'refunded')
        "#,
    )
    .bind(status)
    .bind(&event.data)
    .bind(amount.map(|a| a as i32))
    .bind(currency)
    .bind(failure_code)
    .bind(failure_message)
    .bind(payment_intent_id)
    .bind(app_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update charge: {}", e))?;

    if updated.rows_affected() > 0 {
        tracing::info!("Updated charge via tilled_charge_id={}", payment_intent_id);
        return Ok(());
    }

    // Check if already terminal (idempotent replay — no-op).
    let already_terminal = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM ar_charges WHERE tilled_charge_id = $1 AND app_id = $2 AND status IN ('succeeded', 'failed', 'refunded')",
    )
    .bind(payment_intent_id)
    .bind(app_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("Failed to check terminal charge: {}", e))?;

    if already_terminal.is_some() {
        tracing::info!(
            "Charge {} already terminal, skipping out-of-order event",
            payment_intent_id
        );
        return Ok(());
    }

    // Try to bind a pending charge with NULL tilled_charge_id by customer.
    let tilled_customer_id = pi_data.get("customer").and_then(|v| v.as_str());

    if let Some(cust_id) = tilled_customer_id {
        let bound = sqlx::query(
            r#"
            UPDATE ar_charges
            SET tilled_charge_id = $1, status = $2, metadata = $3,
                amount_cents = COALESCE($4, amount_cents),
                currency = $5,
                failure_code = $6, failure_message = $7,
                updated_at = NOW()
            WHERE app_id = $8 AND tilled_charge_id IS NULL AND status = 'pending'
              AND ar_customer_id = (
                  SELECT id FROM ar_customers
                  WHERE tilled_customer_id = $9 AND app_id = $8 LIMIT 1
              )
            "#,
        )
        .bind(payment_intent_id)
        .bind(status)
        .bind(&event.data)
        .bind(amount.map(|a| a as i32))
        .bind(currency)
        .bind(failure_code)
        .bind(failure_message)
        .bind(app_id)
        .bind(cust_id)
        .execute(db)
        .await
        .map_err(|e| format!("Failed to bind pending charge: {}", e))?;

        if bound.rows_affected() > 0 {
            tracing::info!(
                "Bound pending charge to payment_intent={}",
                payment_intent_id
            );
            return Ok(());
        }
    }

    tracing::warn!(
        "No matching charge for payment_intent={} (may not exist locally yet)",
        payment_intent_id
    );
    Ok(())
}

/// Process charge webhook events.
///
/// Out-of-order guard: `succeeded`, `failed`, `refunded` are terminal.
/// Maps `failure_code` and `failure_message` from webhook data.
pub(super) async fn process_charge_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let charge_data = &event.data;
    let tilled_charge_id = charge_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing charge ID".to_string())?;

    let status = match event.event_type.as_str() {
        "charge.succeeded" => "succeeded",
        "charge.failed" => "failed",
        "charge.refunded" => "refunded",
        _ => "pending",
    };

    let failure_code = charge_data.get("failure_code").and_then(|v| v.as_str());
    let failure_message = charge_data.get("failure_message").and_then(|v| v.as_str());

    // Out-of-order guard: terminal states cannot be regressed.
    let updated = sqlx::query(
        r#"
        UPDATE ar_charges
        SET status = $1, metadata = $2,
            failure_code = COALESCE($3, failure_code),
            failure_message = COALESCE($4, failure_message),
            updated_at = NOW()
        WHERE tilled_charge_id = $5 AND app_id = $6
          AND status NOT IN ('succeeded', 'failed', 'refunded')
        "#,
    )
    .bind(status)
    .bind(&event.data)
    .bind(failure_code)
    .bind(failure_message)
    .bind(tilled_charge_id)
    .bind(app_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update charge: {}", e))?;

    if updated.rows_affected() == 0 {
        tracing::info!(
            "Charge {} already terminal or not found, skipping",
            tilled_charge_id
        );
    } else {
        tracing::info!("Processed charge event for {}", tilled_charge_id);
    }
    Ok(())
}

/// Process invoice webhook events.
///
/// Out-of-order guard: `paid` is terminal — cannot be regressed to `unpaid`
/// or `open` by a late-arriving event.
pub(super) async fn process_invoice_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let invoice_data = &event.data;
    let tilled_invoice_id = invoice_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing invoice ID".to_string())?;

    let status = match event.event_type.as_str() {
        "invoice.created" => "open",
        "invoice.payment_succeeded" => "paid",
        "invoice.payment_failed" => "unpaid",
        _ => "open",
    };

    // Out-of-order guard: paid is terminal.
    let updated = sqlx::query(
        r#"
        UPDATE ar_invoices
        SET status = $1, metadata = $2,
            paid_at = CASE WHEN $1 = 'paid' THEN COALESCE(paid_at, NOW()) ELSE paid_at END,
            updated_at = NOW()
        WHERE tilled_invoice_id = $3 AND app_id = $4
          AND status NOT IN ('paid', 'void', 'written_off')
        "#,
    )
    .bind(status)
    .bind(&event.data)
    .bind(tilled_invoice_id)
    .bind(app_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update invoice: {}", e))?;

    if updated.rows_affected() == 0 {
        tracing::info!(
            "Invoice {} already terminal or not found, skipping",
            tilled_invoice_id
        );
    } else {
        tracing::info!("Processed invoice event for {}", tilled_invoice_id);
    }
    Ok(())
}
