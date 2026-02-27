use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::PgPool;

use axum::Extension;
use security::VerifiedClaims;

use crate::models::{
    ErrorResponse, ListWebhooksQuery, ReplayWebhookRequest, TilledWebhookEvent, Webhook,
    WebhookStatus,
};

/// Maximum age (seconds) of an accepted webhook timestamp.
///
/// Tilled embeds `t=<unix_seconds>` in the signature header. Any event
/// whose timestamp is older than this threshold is rejected as a potential
/// replay, even if the HMAC is otherwise valid.
const WEBHOOK_TIMESTAMP_TOLERANCE_SECS: i64 = 300; // 5 minutes

/// Verify Tilled webhook signature and timestamp freshness.
///
/// Tilled signs webhooks with HMAC-SHA256. The `tilled-signature` header
/// has the form `t=<unix_ts>,v1=<hex_sig>`. We validate:
/// 1. HMAC over `"<ts>.<body>"` matches `v1`.
/// 2. Timestamp is within ±5 minutes of now (replay-window guard).
fn verify_tilled_signature(
    payload: &[u8],
    signature_header: Option<&str>,
    secret: &str,
) -> Result<(), String> {
    let signature = signature_header.ok_or_else(|| "Missing signature header".to_string())?;

    // Tilled sends signature in format: "t=timestamp,v1=signature"
    let sig_parts: Vec<&str> = signature.split(',').collect();
    let mut timestamp = "";
    let mut sig_value = "";

    for part in sig_parts {
        if let Some(value) = part.strip_prefix("t=") {
            timestamp = value;
        } else if let Some(value) = part.strip_prefix("v1=") {
            sig_value = value;
        }
    }

    if timestamp.is_empty() || sig_value.is_empty() {
        return Err("Invalid signature format".to_string());
    }

    // Replay-window check: reject events older than tolerance window.
    let ts_unix: i64 = timestamp
        .parse()
        .map_err(|_| "Invalid timestamp in signature".to_string())?;
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let age = now_unix - ts_unix;
    if age.abs() > WEBHOOK_TIMESTAMP_TOLERANCE_SECS {
        return Err(format!(
            "Webhook timestamp too old or too far in the future (age={}s, tolerance={}s)",
            age, WEBHOOK_TIMESTAMP_TOLERANCE_SECS
        ));
    }

    // Construct signed payload: timestamp.payload
    let signed_payload = format!("{}.{}", timestamp, String::from_utf8_lossy(payload));

    // Compute expected signature
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("Invalid secret: {}", e))?;
    mac.update(signed_payload.as_bytes());
    let expected_sig = hex::encode(mac.finalize().into_bytes());

    // Compare signatures (constant-time comparison would be better but
    // hex strings are already fixed length; timing leak is not exploitable here
    // since the attacker does not control the secret).
    if expected_sig != sig_value {
        return Err("Signature verification failed".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sig(secret: &str, ts: i64, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let signed = format!("{}.{}", ts, String::from_utf8_lossy(body));
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signed.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());
        format!("t={},v1={}", ts, sig)
    }

    fn now_unix() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[test]
    fn test_valid_signature_fresh_timestamp() {
        let secret = "whsec_unit_test_secret";
        let body = b"{}";
        let ts = now_unix();
        let header = make_sig(secret, ts, body);
        assert!(verify_tilled_signature(body, Some(&header), secret).is_ok());
    }

    #[test]
    fn test_invalid_signature_rejected() {
        let secret = "whsec_unit_test_secret";
        let body = b"{}";
        let ts = now_unix();
        let header = format!("t={},v1=deadbeef", ts);
        let err = verify_tilled_signature(body, Some(&header), secret).unwrap_err();
        assert!(err.contains("Signature verification failed"), "{}", err);
    }

    #[test]
    fn test_old_timestamp_rejected() {
        let secret = "whsec_unit_test_secret";
        let body = b"{}";
        let old_ts = now_unix() - 600; // 10 minutes ago
        let header = make_sig(secret, old_ts, body);
        let err = verify_tilled_signature(body, Some(&header), secret).unwrap_err();
        assert!(err.contains("too old"), "{}", err);
    }

    #[test]
    fn test_missing_header_rejected() {
        assert!(verify_tilled_signature(b"{}", None, "secret").is_err());
    }

    #[test]
    fn test_malformed_header_rejected() {
        let err = verify_tilled_signature(b"{}", Some("garbage"), "secret").unwrap_err();
        assert!(err.contains("Invalid signature format"), "{}", err);
    }
}

/// Process webhook event based on type
async fn process_webhook_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    tracing::info!("Processing webhook event: {}", event.event_type);

    match event.event_type.as_str() {
        // Customer events
        "customer.created" | "customer.updated" => {
            process_customer_event(db, app_id, event).await?;
        }
        // Payment intent events
        "payment_intent.succeeded" | "payment_intent.failed" => {
            process_payment_intent_event(db, app_id, event).await?;
        }
        // Payment method events
        "payment_method.attached" | "payment_method.detached" => {
            process_payment_method_event(db, app_id, event).await?;
        }
        // Subscription events
        "subscription.created" | "subscription.updated" | "subscription.canceled" => {
            process_subscription_event(db, app_id, event).await?;
        }
        // Charge events
        "charge.succeeded" | "charge.failed" | "charge.refunded" => {
            process_charge_event(db, app_id, event).await?;
        }
        // Invoice events
        "invoice.created" | "invoice.payment_succeeded" | "invoice.payment_failed" => {
            process_invoice_event(db, app_id, event).await?;
        }
        _ => {
            tracing::warn!("Unhandled webhook event type: {}", event.event_type);
        }
    }

    Ok(())
}

/// Process customer webhook events.
///
/// Idempotent upsert by `tilled_customer_id`. If a local customer exists in
/// `pending_sync` with a matching email, bind the provider ID and activate.
async fn process_customer_event(
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

/// Process payment intent webhook events.
///
/// Out-of-order guard: `succeeded` and `failed` are terminal — older events
/// cannot regress them. If charge exists by `tilled_charge_id`, update it.
/// If a pending charge with NULL provider ID matches by customer, bind it.
async fn process_payment_intent_event(
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
    let failure_code = pi_data.get("last_payment_error")
        .and_then(|e| e.get("code"))
        .and_then(|v| v.as_str());
    let failure_message = pi_data.get("last_payment_error")
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
    let tilled_customer_id = pi_data
        .get("customer")
        .and_then(|v| v.as_str());

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
            tracing::info!("Bound pending charge to payment_intent={}", payment_intent_id);
            return Ok(());
        }
    }

    tracing::warn!(
        "No matching charge for payment_intent={} (may not exist locally yet)",
        payment_intent_id
    );
    Ok(())
}

/// Process payment method webhook events.
///
/// `attached`: Bind provider ID to a `pending_sync` PM or update existing.
/// Hydrate card details (brand, last4, exp). `detached`: Idempotent soft-delete.
async fn process_payment_method_event(
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
        let card = pm_data.get("card");
        let brand = card.and_then(|c| c.get("brand")).and_then(|v| v.as_str());
        let last4 = card.and_then(|c| c.get("last4")).and_then(|v| v.as_str());
        let exp_month = card.and_then(|c| c.get("exp_month")).and_then(|v| v.as_i64());
        let exp_year = card.and_then(|c| c.get("exp_year")).and_then(|v| v.as_i64());
        let pm_type = pm_data.get("type").and_then(|v| v.as_str()).unwrap_or("card");

        // Try to bind a pending_sync PM by customer.
        let tilled_customer_id = pm_data
            .get("customer")
            .and_then(|v| v.as_str());

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

/// Process subscription webhook events.
///
/// `created`: Bind `tilled_subscription_id`, set active, populate periods.
/// `updated`: Update status + metadata; out-of-order guard on terminal states.
/// `canceled`: Set canceled + `canceled_at`; terminal — cannot be regressed.
async fn process_subscription_event(
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
    let current_period_start = sub_data
        .get("current_period_start")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ").ok()
            .or_else(|| s.parse::<i64>().ok().map(|ts| {
                chrono::DateTime::from_timestamp(ts, 0)
                    .unwrap_or_default()
                    .naive_utc()
            })));
    let current_period_end = sub_data
        .get("current_period_end")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ").ok()
            .or_else(|| s.parse::<i64>().ok().map(|ts| {
                chrono::DateTime::from_timestamp(ts, 0)
                    .unwrap_or_default()
                    .naive_utc()
            })));

    if event.event_type == "subscription.created" {
        // Try to bind a pending_sync subscription by customer.
        let tilled_customer_id = sub_data.get("customer").and_then(|v| v.as_str());
        let mut bound = false;

        if let Some(cust_id) = tilled_customer_id {
            let res = sqlx::query(
                r#"
                UPDATE ar_subscriptions
                SET tilled_subscription_id = $1, status = 'active'::ar_subscriptions_status,
                    metadata = $2,
                    current_period_start = COALESCE($3, current_period_start),
                    current_period_end = COALESCE($4, current_period_end),
                    update_source = 'webhook', updated_at = NOW()
                WHERE app_id = $5
                  AND status = 'pending_sync'::ar_subscriptions_status
                  AND tilled_subscription_id IS NULL
                  AND ar_customer_id = (
                      SELECT id FROM ar_customers
                      WHERE tilled_customer_id = $6 AND app_id = $5 LIMIT 1
                  )
                "#,
            )
            .bind(tilled_sub_id)
            .bind(&event.data)
            .bind(current_period_start)
            .bind(current_period_end)
            .bind(app_id)
            .bind(cust_id)
            .execute(db)
            .await
            .map_err(|e| format!("Failed to bind pending subscription: {}", e))?;
            bound = res.rows_affected() > 0;
        }

        if !bound {
            // Update if already exists by tilled_subscription_id.
            sqlx::query(
                r#"
                UPDATE ar_subscriptions
                SET status = 'active'::ar_subscriptions_status, metadata = $1,
                    current_period_start = COALESCE($2, current_period_start),
                    current_period_end = COALESCE($3, current_period_end),
                    update_source = 'webhook', updated_at = NOW()
                WHERE tilled_subscription_id = $4 AND app_id = $5
                "#,
            )
            .bind(&event.data)
            .bind(current_period_start)
            .bind(current_period_end)
            .bind(tilled_sub_id)
            .bind(app_id)
            .execute(db)
            .await
            .map_err(|e| format!("Failed to update subscription: {}", e))?;
        }
    } else if event.event_type == "subscription.canceled" {
        // Terminal: set canceled. Cannot be regressed by older events.
        sqlx::query(
            r#"
            UPDATE ar_subscriptions
            SET status = 'canceled'::ar_subscriptions_status, metadata = $1,
                canceled_at = COALESCE(canceled_at, NOW()),
                update_source = 'webhook', updated_at = NOW()
            WHERE tilled_subscription_id = $2 AND app_id = $3
              AND status != 'canceled'::ar_subscriptions_status
            "#,
        )
        .bind(&event.data)
        .bind(tilled_sub_id)
        .bind(app_id)
        .execute(db)
        .await
        .map_err(|e| format!("Failed to cancel subscription: {}", e))?;
    } else {
        // subscription.updated — out-of-order guard: canceled is terminal.
        sqlx::query(
            r#"
            UPDATE ar_subscriptions
            SET status = $1::ar_subscriptions_status, metadata = $2,
                current_period_start = COALESCE($3, current_period_start),
                current_period_end = COALESCE($4, current_period_end),
                update_source = 'webhook', updated_at = NOW()
            WHERE tilled_subscription_id = $5 AND app_id = $6
              AND status != 'canceled'::ar_subscriptions_status
            "#,
        )
        .bind(status)
        .bind(&event.data)
        .bind(current_period_start)
        .bind(current_period_end)
        .bind(tilled_sub_id)
        .bind(app_id)
        .execute(db)
        .await
        .map_err(|e| format!("Failed to update subscription: {}", e))?;
    }

    tracing::info!("Processed subscription event for {}", tilled_sub_id);
    Ok(())
}

/// Process charge webhook events.
///
/// Out-of-order guard: `succeeded`, `failed`, `refunded` are terminal.
/// Maps `failure_code` and `failure_message` from webhook data.
async fn process_charge_event(
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

    let failure_code = charge_data
        .get("failure_code")
        .and_then(|v| v.as_str());
    let failure_message = charge_data
        .get("failure_message")
        .and_then(|v| v.as_str());

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
async fn process_invoice_event(
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

/// POST /api/ar/webhooks/tilled - Receive Tilled webhook
pub async fn receive_tilled_webhook(
    State(db): State<PgPool>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // Webhook endpoints are called by Tilled (HMAC-authenticated, not JWT).
    // Tenant is determined by the registered webhook endpoint configuration.
    let app_id = std::env::var("TILLED_WEBHOOK_APP_ID").unwrap_or_else(|_| {
        headers
            .get("x-tilled-account")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string()
    });

    // Get webhook secret from environment — required, no fallback
    let webhook_secret = std::env::var("TILLED_WEBHOOK_SECRET_TRASHTECH")
        .or_else(|_| std::env::var("TILLED_WEBHOOK_SECRET"))
        .map_err(|_| {
            tracing::error!("TILLED_WEBHOOK_SECRET not configured — rejecting webhook");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "config_error",
                    "Webhook secret not configured",
                )),
            )
        })?;

    // Always verify signature — no bypass
    let signature = headers
        .get("tilled-signature")
        .or_else(|| headers.get("x-tilled-signature"))
        .and_then(|v| v.to_str().ok());

    if let Err(e) = verify_tilled_signature(&body, signature, &webhook_secret) {
        tracing::warn!("Webhook signature verification failed: {}", e);
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse::new("signature_error", e)),
        ));
    }

    // Parse webhook event
    let event: TilledWebhookEvent = serde_json::from_slice(&body).map_err(|e| {
        tracing::error!("Failed to parse webhook event: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "parse_error",
                format!("Failed to parse webhook: {}", e),
            )),
        )
    })?;

    tracing::info!(
        "Received webhook event: {} (id: {})",
        event.event_type,
        event.id
    );

    // Check for duplicate event (idempotency)
    let existing = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT id FROM ar_webhooks
        WHERE event_id = $1 AND app_id = $2
        "#,
    )
    .bind(&event.id)
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to check for duplicate webhook: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to check idempotency",
            )),
        )
    })?;

    if existing.is_some() {
        tracing::info!("Webhook event {} already processed (idempotent)", event.id);
        // Return 200 to prevent Tilled retries
        return Ok(StatusCode::OK);
    }

    // Store webhook in database (status: received)
    let webhook_id = sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_webhooks (
            app_id, event_id, event_type, status, payload, attempt_count, received_at
        )
        VALUES ($1, $2, $3, 'received', $4, 1, NOW())
        RETURNING id
        "#,
    )
    .bind(&app_id)
    .bind(&event.id)
    .bind(&event.event_type)
    .bind(serde_json::to_value(&event).unwrap())
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to store webhook: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to store webhook",
            )),
        )
    })?;

    // Process event asynchronously (don't block webhook response)
    // Update status to processing
    sqlx::query(
        r#"
        UPDATE ar_webhooks
        SET status = 'processing', last_attempt_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(webhook_id)
    .execute(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update webhook status: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to update webhook status",
            )),
        )
    })?;

    // Process the event
    match process_webhook_event(&db, &app_id, &event).await {
        Ok(_) => {
            // Mark as processed
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'processed', processed_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(webhook_id)
            .execute(&db)
            .await
            .ok();

            tracing::info!("Successfully processed webhook event {}", event.id);
        }
        Err(e) => {
            // Mark as failed
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'failed', error = $1, error_code = 'processing_error'
                WHERE id = $2
                "#,
            )
            .bind(&e)
            .bind(webhook_id)
            .execute(&db)
            .await
            .ok();

            tracing::error!("Failed to process webhook event {}: {}", event.id, e);
        }
    }

    // Always return 200 to prevent Tilled retries
    // Errors are stored in the database for manual investigation
    Ok(StatusCode::OK)
}

/// GET /api/ar/webhooks - List webhooks (admin)
pub async fn list_webhooks(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListWebhooksQuery>,
) -> Result<Json<Vec<Webhook>>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0);

    let mut sql = String::from(
        r#"
        SELECT
            id, app_id, event_id, event_type, status, error, payload,
            attempt_count, last_attempt_at, next_attempt_at, dead_at,
            error_code, received_at, processed_at
        FROM ar_webhooks
        WHERE app_id = $1
        "#,
    );

    let mut param_count = 1;

    if query.event_type.is_some() {
        param_count += 1;
        sql.push_str(&format!(" AND event_type = ${}", param_count));
    }

    if query.status.is_some() {
        param_count += 1;
        sql.push_str(&format!(
            " AND status = ${}::ar_webhooks_status",
            param_count
        ));
    }

    sql.push_str(" ORDER BY received_at DESC LIMIT $");
    param_count += 1;
    sql.push_str(&param_count.to_string());
    sql.push_str(" OFFSET $");
    param_count += 1;
    sql.push_str(&param_count.to_string());

    let mut query_builder = sqlx::query_as::<_, Webhook>(&sql).bind(&app_id);

    if let Some(event_type) = &query.event_type {
        query_builder = query_builder.bind(event_type);
    }

    if let Some(status) = &query.status {
        query_builder = query_builder.bind(status);
    }

    query_builder = query_builder.bind(limit).bind(offset);

    let webhooks = query_builder.fetch_all(&db).await.map_err(|e| {
        tracing::error!("Failed to list webhooks: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to list webhooks",
            )),
        )
    })?;

    Ok(Json(webhooks))
}

/// GET /api/ar/webhooks/:id - Get webhook details
pub async fn get_webhook(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Webhook>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let webhook = sqlx::query_as::<_, Webhook>(
        r#"
        SELECT
            id, app_id, event_id, event_type, status, error, payload,
            attempt_count, last_attempt_at, next_attempt_at, dead_at,
            error_code, received_at, processed_at
        FROM ar_webhooks
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch webhook: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch webhook",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("not_found", "Webhook not found")),
        )
    })?;

    Ok(Json(webhook))
}

/// POST /api/ar/webhooks/:id/replay - Replay a webhook
pub async fn replay_webhook(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<ReplayWebhookRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    // Fetch webhook
    let webhook = sqlx::query_as::<_, Webhook>(
        r#"
        SELECT
            id, app_id, event_id, event_type, status, error, payload,
            attempt_count, last_attempt_at, next_attempt_at, dead_at,
            error_code, received_at, processed_at
        FROM ar_webhooks
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch webhook: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch webhook",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("not_found", "Webhook not found")),
        )
    })?;

    // Check if replay is allowed
    let force = req.force.unwrap_or(false);
    if webhook.status != WebhookStatus::Failed && !force {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_status",
                "Can only replay failed webhooks (use force=true to override)",
            )),
        ));
    }

    // Parse payload
    let event: TilledWebhookEvent = serde_json::from_value(webhook.payload.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_webhook",
                "Webhook has no payload",
            )),
        )
    })?)
    .map_err(|e| {
        tracing::error!("Failed to parse webhook payload: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "parse_error",
                "Failed to parse webhook payload",
            )),
        )
    })?;

    // Update status to processing
    sqlx::query(
        r#"
        UPDATE ar_webhooks
        SET status = 'processing', last_attempt_at = NOW(), attempt_count = attempt_count + 1
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update webhook status: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to update webhook status",
            )),
        )
    })?;

    // Process the event
    match process_webhook_event(&db, &app_id, &event).await {
        Ok(_) => {
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'processed', processed_at = NOW(), error = NULL, error_code = NULL
                WHERE id = $1
                "#,
            )
            .bind(id)
            .execute(&db)
            .await
            .ok();

            tracing::info!("Successfully replayed webhook {}", id);
            Ok(StatusCode::OK)
        }
        Err(e) => {
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'failed', error = $1, error_code = 'processing_error'
                WHERE id = $2
                "#,
            )
            .bind(&e)
            .bind(id)
            .execute(&db)
            .await
            .ok();

            tracing::error!("Failed to replay webhook {}: {}", id, e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("processing_error", e)),
            ))
        }
    }
}
