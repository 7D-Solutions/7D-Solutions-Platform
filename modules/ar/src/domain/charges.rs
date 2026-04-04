//! Charge repository — all SQL operations for the charges domain.

use serde_json::Value as JsonValue;
use sqlx::PgExecutor;

use crate::models::Charge;

// ============================================================================
// Reads
// ============================================================================

/// Fetch a charge by ID with tenant isolation.
pub async fn fetch_with_tenant<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    app_id: &str,
) -> Result<Option<Charge>, sqlx::Error> {
    sqlx::query_as::<_, Charge>(
        r#"
        SELECT
            ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.ar_customer_id, ch.subscription_id,
            ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
            ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
            ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
            ch.created_at, ch.updated_at
        FROM ar_charges ch
        INNER JOIN ar_customers c ON ch.ar_customer_id = c.id
        WHERE ch.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Find a charge by reference_id and app_id (idempotency check).
pub async fn find_by_reference_id<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    reference_id: &str,
) -> Result<Option<Charge>, sqlx::Error> {
    sqlx::query_as::<_, Charge>(
        r#"
        SELECT
            id, app_id, tilled_charge_id, invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, failure_code, failure_message,
            product_type, quantity, service_frequency, weight_amount, location_reference,
            created_at, updated_at
        FROM ar_charges
        WHERE app_id = $1 AND reference_id = $2
        "#,
    )
    .bind(app_id)
    .bind(reference_id)
    .fetch_optional(executor)
    .await
}

/// Count charges matching filters.
pub async fn count_charges<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: Option<i32>,
    invoice_id: Option<i32>,
    status: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let mut sql = String::from(
        "SELECT COUNT(*) FROM ar_charges ch \
         INNER JOIN ar_customers c ON ch.ar_customer_id = c.id \
         WHERE c.app_id = $1",
    );
    let mut bind_idx = 2;
    if customer_id.is_some() {
        sql.push_str(&format!(" AND ch.ar_customer_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if invoice_id.is_some() {
        sql.push_str(&format!(" AND ch.invoice_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if status.is_some() {
        sql.push_str(&format!(" AND ch.status = ${bind_idx}"));
    }
    let mut q = sqlx::query_scalar::<_, i64>(&sql).bind(app_id);
    if let Some(cid) = customer_id {
        q = q.bind(cid);
    }
    if let Some(iid) = invoice_id {
        q = q.bind(iid);
    }
    if let Some(st) = status {
        q = q.bind(st);
    }
    q.fetch_one(executor).await
}

/// List charges with optional filters and pagination.
pub async fn list_charges<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: Option<i32>,
    invoice_id: Option<i32>,
    status: Option<&str>,
    limit: i32,
    offset: i32,
) -> Result<Vec<Charge>, sqlx::Error> {
    let mut sql = String::from(
        r#"SELECT
            ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.ar_customer_id, ch.subscription_id,
            ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
            ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
            ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
            ch.created_at, ch.updated_at
        FROM ar_charges ch
        INNER JOIN ar_customers c ON ch.ar_customer_id = c.id
        WHERE c.app_id = $1"#,
    );
    let mut idx = 2;
    if customer_id.is_some() {
        sql.push_str(&format!(" AND ch.ar_customer_id = ${idx}"));
        idx += 1;
    }
    if invoice_id.is_some() {
        sql.push_str(&format!(" AND ch.invoice_id = ${idx}"));
        idx += 1;
    }
    if status.is_some() {
        sql.push_str(&format!(" AND ch.status = ${idx}"));
        idx += 1;
    }
    sql.push_str(&format!(
        " ORDER BY ch.created_at DESC LIMIT ${idx} OFFSET ${}",
        idx + 1
    ));

    let mut q = sqlx::query_as::<_, Charge>(&sql).bind(app_id);
    if let Some(cid) = customer_id {
        q = q.bind(cid);
    }
    if let Some(iid) = invoice_id {
        q = q.bind(iid);
    }
    if let Some(st) = status {
        q = q.bind(st);
    }
    q.bind(limit).bind(offset).fetch_all(executor).await
}

// ============================================================================
// Writes
// ============================================================================

/// Insert a new pending charge.
#[allow(clippy::too_many_arguments)]
pub async fn insert_charge<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: i32,
    amount_cents: i64,
    currency: &str,
    charge_type: &str,
    reason: &str,
    reference_id: &str,
    service_date: Option<chrono::NaiveDate>,
    note: Option<String>,
    metadata: Option<JsonValue>,
) -> Result<Charge, sqlx::Error> {
    sqlx::query_as::<_, Charge>(
        r#"
        INSERT INTO ar_charges (
            app_id, ar_customer_id, subscription_id, invoice_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, tilled_charge_id,
            created_at, updated_at
        )
        VALUES ($1, $2, NULL, NULL, 'pending', $3, $4, $5, $6, $7, $8, $9, $10, NULL, NOW(), NOW())
        RETURNING
            id, app_id, tilled_charge_id, invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, failure_code, failure_message,
            product_type, quantity, service_frequency, weight_amount, location_reference,
            created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(amount_cents)
    .bind(currency)
    .bind(charge_type)
    .bind(reason)
    .bind(reference_id)
    .bind(service_date)
    .bind(note)
    .bind(metadata)
    .fetch_one(executor)
    .await
}

/// Update a charge after capture.
pub async fn update_after_capture<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    status: &str,
    amount_cents: i64,
) -> Result<Charge, sqlx::Error> {
    sqlx::query_as::<_, Charge>(
        r#"
        UPDATE ar_charges
        SET status = $1, amount_cents = $2, updated_at = NOW()
        WHERE id = $3
        RETURNING
            id, app_id, tilled_charge_id, invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, failure_code, failure_message,
            product_type, quantity, service_frequency, weight_amount, location_reference,
            created_at, updated_at
        "#,
    )
    .bind(status)
    .bind(amount_cents)
    .bind(id)
    .fetch_one(executor)
    .await
}

// ============================================================================
// Webhook operations
// ============================================================================

/// Update a charge by tilled_charge_id (payment_intent webhook). Out-of-order guard.
#[allow(clippy::too_many_arguments)]
pub async fn webhook_update_by_tilled_id<'e>(
    executor: impl PgExecutor<'e>,
    status: &str,
    data: &JsonValue,
    amount: Option<i32>,
    currency: &str,
    failure_code: Option<&str>,
    failure_message: Option<&str>,
    tilled_charge_id: &str,
    app_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
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
    .bind(data)
    .bind(amount)
    .bind(currency)
    .bind(failure_code)
    .bind(failure_message)
    .bind(tilled_charge_id)
    .bind(app_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Check if a charge is already in a terminal state.
pub async fn check_terminal<'e>(
    executor: impl PgExecutor<'e>,
    tilled_charge_id: &str,
    app_id: &str,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM ar_charges WHERE tilled_charge_id = $1 AND app_id = $2 AND status IN ('succeeded', 'failed', 'refunded')",
    )
    .bind(tilled_charge_id)
    .bind(app_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.is_some())
}

/// Bind a pending charge (NULL tilled_charge_id) to a payment intent by customer.
#[allow(clippy::too_many_arguments)]
pub async fn bind_pending_by_customer<'e>(
    executor: impl PgExecutor<'e>,
    payment_intent_id: &str,
    status: &str,
    data: &JsonValue,
    amount: Option<i32>,
    currency: &str,
    failure_code: Option<&str>,
    failure_message: Option<&str>,
    app_id: &str,
    tilled_customer_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
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
    .bind(data)
    .bind(amount)
    .bind(currency)
    .bind(failure_code)
    .bind(failure_message)
    .bind(app_id)
    .bind(tilled_customer_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Update charge status via charge event webhook. Out-of-order guard.
pub async fn webhook_update_charge_event<'e>(
    executor: impl PgExecutor<'e>,
    status: &str,
    data: &JsonValue,
    failure_code: Option<&str>,
    failure_message: Option<&str>,
    tilled_charge_id: &str,
    app_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
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
    .bind(data)
    .bind(failure_code)
    .bind(failure_message)
    .bind(tilled_charge_id)
    .bind(app_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Update invoice status via invoice event webhook. Out-of-order guard.
pub async fn webhook_update_invoice_event<'e>(
    executor: impl PgExecutor<'e>,
    status: &str,
    data: &JsonValue,
    tilled_invoice_id: &str,
    app_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
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
    .bind(data)
    .bind(tilled_invoice_id)
    .bind(app_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}
