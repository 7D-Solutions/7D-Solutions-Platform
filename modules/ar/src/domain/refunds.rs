//! Refund repository — all SQL operations for the refunds domain.

use serde_json::Value as JsonValue;
use sqlx::PgExecutor;

use crate::models::{Charge, Refund};

// ============================================================================
// Reads
// ============================================================================

/// Find an existing refund by reference_id (idempotency).
pub async fn find_by_reference_id<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    reference_id: &str,
) -> Result<Option<Refund>, sqlx::Error> {
    sqlx::query_as::<_, Refund>(
        r#"
        SELECT
            id, app_id, ar_customer_id, charge_id, tilled_refund_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            failure_code, failure_message, created_at, updated_at
        FROM ar_refunds
        WHERE app_id = $1 AND reference_id = $2
        "#,
    )
    .bind(app_id)
    .bind(reference_id)
    .fetch_optional(executor)
    .await
}

/// Load a charge by ID with app_id scoping (used by refund create).
pub async fn fetch_charge_for_refund<'e>(
    executor: impl PgExecutor<'e>,
    charge_id: i32,
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
    .bind(charge_id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Calculate total already refunded for a charge.
pub async fn sum_refunded<'e>(
    executor: impl PgExecutor<'e>,
    charge_id: i32,
    app_id: &str,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(amount_cents)::BIGINT, 0)
        FROM ar_refunds
        WHERE charge_id = $1 AND app_id = $2 AND status IN ('pending', 'succeeded')
        "#,
    )
    .bind(charge_id)
    .bind(app_id)
    .fetch_one(executor)
    .await
}

/// Fetch a single refund by ID with tenant isolation.
pub async fn fetch_by_id<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    app_id: &str,
) -> Result<Option<Refund>, sqlx::Error> {
    sqlx::query_as::<_, Refund>(
        r#"
        SELECT
            id, app_id, ar_customer_id, charge_id, tilled_refund_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            failure_code, failure_message, created_at, updated_at
        FROM ar_refunds
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Count refunds matching filters.
pub async fn count_refunds<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    charge_id: Option<i32>,
    customer_id: Option<i32>,
    status: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let mut sql = String::from("SELECT COUNT(*) FROM ar_refunds WHERE app_id = $1");
    let mut idx = 2;
    if charge_id.is_some() {
        sql.push_str(&format!(" AND charge_id = ${idx}"));
        idx += 1;
    }
    if customer_id.is_some() {
        sql.push_str(&format!(" AND ar_customer_id = ${idx}"));
        idx += 1;
    }
    if status.is_some() {
        sql.push_str(&format!(" AND status = ${idx}"));
    }
    let mut q = sqlx::query_scalar::<_, i64>(&sql).bind(app_id);
    if let Some(cid) = charge_id {
        q = q.bind(cid);
    }
    if let Some(cust_id) = customer_id {
        q = q.bind(cust_id);
    }
    if let Some(st) = status {
        q = q.bind(st);
    }
    q.fetch_one(executor).await
}

/// List refunds with optional filters and pagination.
pub async fn list_refunds<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    charge_id: Option<i32>,
    customer_id: Option<i32>,
    status: Option<&str>,
    limit: i32,
    offset: i32,
) -> Result<Vec<Refund>, sqlx::Error> {
    let mut sql = String::from(
        r#"
        SELECT
            id, app_id, ar_customer_id, charge_id, tilled_refund_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            failure_code, failure_message, created_at, updated_at
        FROM ar_refunds
        WHERE app_id = $1
        "#,
    );

    let mut idx = 2;
    if charge_id.is_some() {
        sql.push_str(&format!(" AND charge_id = ${idx}"));
        idx += 1;
    }
    if customer_id.is_some() {
        sql.push_str(&format!(" AND ar_customer_id = ${idx}"));
        idx += 1;
    }
    if status.is_some() {
        sql.push_str(&format!(" AND status = ${idx}"));
        idx += 1;
    }

    sql.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${idx} OFFSET ${}",
        idx + 1
    ));

    let mut q = sqlx::query_as::<_, Refund>(&sql).bind(app_id);
    if let Some(cid) = charge_id {
        q = q.bind(cid);
    }
    if let Some(cust_id) = customer_id {
        q = q.bind(cust_id);
    }
    if let Some(st) = status {
        q = q.bind(st);
    }
    q.bind(limit).bind(offset).fetch_all(executor).await
}

// ============================================================================
// Writes
// ============================================================================

/// Insert a new pending refund record.
#[allow(clippy::too_many_arguments)]
pub async fn insert_refund<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: i32,
    charge_id: i32,
    tilled_charge_id: &Option<String>,
    amount_cents: i64,
    currency: &str,
    reason: &Option<String>,
    reference_id: &str,
    note: &Option<String>,
    metadata: &Option<JsonValue>,
) -> Result<Refund, sqlx::Error> {
    sqlx::query_as::<_, Refund>(
        r#"
        INSERT INTO ar_refunds (
            app_id, ar_customer_id, charge_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, 'pending', $5, $6, $7, $8, $9, $10, NOW(), NOW())
        RETURNING
            id, app_id, ar_customer_id, charge_id, tilled_refund_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            failure_code, failure_message, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(charge_id)
    .bind(tilled_charge_id)
    .bind(amount_cents)
    .bind(currency)
    .bind(reason)
    .bind(reference_id)
    .bind(note)
    .bind(metadata)
    .fetch_one(executor)
    .await
}

/// Update a refund after provider call (set status + tilled_refund_id).
pub async fn update_after_provider<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    status: &str,
    tilled_refund_id: &str,
) -> Result<Refund, sqlx::Error> {
    sqlx::query_as::<_, Refund>(
        r#"
        UPDATE ar_refunds
        SET status = $1, tilled_refund_id = $2, updated_at = NOW()
        WHERE id = $3
        RETURNING
            id, app_id, ar_customer_id, charge_id, tilled_refund_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            failure_code, failure_message, created_at, updated_at
        "#,
    )
    .bind(status)
    .bind(tilled_refund_id)
    .bind(id)
    .fetch_one(executor)
    .await
}
