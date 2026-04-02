//! Invoice repository — all SQL operations for the invoices domain.

use chrono::NaiveDateTime;
use serde_json::Value as JsonValue;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::models::{Customer, Invoice, ListInvoicesQuery, Subscription};

// ============================================================================
// Guard Reads (pre-mutation checks)
// ============================================================================

/// Fetch a customer by ID and app_id. Used to verify customer exists before
/// creating an invoice.
pub async fn fetch_customer<'e>(
    executor: impl PgExecutor<'e>,
    customer_id: i32,
    app_id: &str,
) -> Result<Option<Customer>, sqlx::Error> {
    sqlx::query_as::<_, Customer>(
        r#"
        SELECT
            id, app_id, external_customer_id, tilled_customer_id, status,
            email, name, default_payment_method_id, payment_method_type,
            metadata, update_source, updated_by, delinquent_since,
            grace_period_end, next_retry_at, retry_attempt_count,
            created_at, updated_at
        FROM ar_customers
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(customer_id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Fetch a subscription by ID, app_id, and customer_id.
pub async fn fetch_subscription<'e>(
    executor: impl PgExecutor<'e>,
    subscription_id: i32,
    app_id: &str,
    customer_id: i32,
) -> Result<Option<Subscription>, sqlx::Error> {
    sqlx::query_as::<_, Subscription>(
        r#"
        SELECT
            s.id, s.app_id, s.ar_customer_id, s.tilled_subscription_id,
            s.plan_id, s.plan_name, s.price_cents, s.status, s.interval_unit, s.interval_count,
            s.billing_cycle_anchor, s.current_period_start, s.current_period_end,
            s.cancel_at_period_end, s.cancel_at, s.canceled_at, s.ended_at,
            s.payment_method_id, s.payment_method_type, s.metadata,
            s.update_source, s.updated_by, s.created_at, s.updated_at
        FROM ar_subscriptions s
        WHERE s.id = $1 AND s.app_id = $2 AND s.ar_customer_id = $3
        "#,
    )
    .bind(subscription_id)
    .bind(app_id)
    .bind(customer_id)
    .fetch_optional(executor)
    .await
}

/// Fetch an invoice for mutation guard checks (without party_id — preserves
/// existing response behavior for update/finalize endpoints).
pub async fn fetch_invoice_for_mutation<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    app_id: &str,
) -> Result<Option<Invoice>, sqlx::Error> {
    sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.correlation_id, i.created_at, i.updated_at
        FROM ar_invoices i
        INNER JOIN ar_customers c ON i.ar_customer_id = c.id
        WHERE i.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Fetch customer's default payment method ID.
pub async fn fetch_customer_default_payment_method<'e>(
    executor: impl PgExecutor<'e>,
    customer_id: i32,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT default_payment_method_id FROM ar_customers WHERE id = $1")
        .bind(customer_id)
        .fetch_optional(executor)
        .await
        .map(|opt| opt.flatten())
}

// ============================================================================
// Query Reads (API responses — include party_id)
// ============================================================================

/// Fetch a single invoice by ID with tenant isolation. Includes party_id.
pub async fn fetch_invoice_with_tenant<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    app_id: &str,
) -> Result<Option<Invoice>, sqlx::Error> {
    sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.correlation_id, i.party_id, i.created_at, i.updated_at
        FROM ar_invoices i
        INNER JOIN ar_customers c ON i.ar_customer_id = c.id
        WHERE i.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Count invoices matching the given filters.
pub async fn count_invoices<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    query: &ListInvoicesQuery,
) -> Result<i64, sqlx::Error> {
    let mut sql = String::from(
        "SELECT COUNT(*) FROM ar_invoices i \
         INNER JOIN ar_customers c ON i.ar_customer_id = c.id \
         WHERE c.app_id = $1",
    );
    let mut bind_idx = 2;
    if query.customer_id.is_some() {
        sql.push_str(&format!(" AND i.ar_customer_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if query.subscription_id.is_some() {
        sql.push_str(&format!(" AND i.subscription_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if query.status.is_some() {
        sql.push_str(&format!(" AND i.status = ${bind_idx}"));
    }

    let mut q = sqlx::query_scalar::<_, i64>(&sql).bind(app_id);
    if let Some(cid) = query.customer_id {
        q = q.bind(cid);
    }
    if let Some(sid) = query.subscription_id {
        q = q.bind(sid);
    }
    if let Some(ref st) = query.status {
        q = q.bind(st);
    }
    q.fetch_one(executor).await
}

/// Fetch a page of invoices matching the given filters. Includes party_id.
pub async fn fetch_invoices_page<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    query: &ListInvoicesQuery,
    limit: i32,
    offset: i32,
) -> Result<Vec<Invoice>, sqlx::Error> {
    let mut sql = String::from(
        r#"SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.correlation_id, i.party_id, i.created_at, i.updated_at
        FROM ar_invoices i
        INNER JOIN ar_customers c ON i.ar_customer_id = c.id
        WHERE c.app_id = $1"#,
    );
    let mut bind_idx = 2;
    if query.customer_id.is_some() {
        sql.push_str(&format!(" AND i.ar_customer_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if query.subscription_id.is_some() {
        sql.push_str(&format!(" AND i.subscription_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if query.status.is_some() {
        sql.push_str(&format!(" AND i.status = ${bind_idx}"));
        bind_idx += 1;
    }
    sql.push_str(&format!(
        " ORDER BY i.created_at DESC LIMIT ${bind_idx} OFFSET ${}",
        bind_idx + 1
    ));

    let mut q = sqlx::query_as::<_, Invoice>(&sql).bind(app_id);
    if let Some(cid) = query.customer_id {
        q = q.bind(cid);
    }
    if let Some(sid) = query.subscription_id {
        q = q.bind(sid);
    }
    if let Some(ref st) = query.status {
        q = q.bind(st);
    }
    q.bind(limit).bind(offset).fetch_all(executor).await
}

// ============================================================================
// Writes
// ============================================================================

/// Insert a new invoice. Must be called within a transaction.
#[allow(clippy::too_many_arguments)]
pub async fn insert_invoice<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    tilled_invoice_id: &str,
    ar_customer_id: i32,
    subscription_id: Option<i32>,
    status: &str,
    amount_cents: i32,
    currency: &str,
    due_at: Option<NaiveDateTime>,
    metadata: Option<JsonValue>,
    billing_period_start: Option<NaiveDateTime>,
    billing_period_end: Option<NaiveDateTime>,
    line_item_details: Option<JsonValue>,
    compliance_codes: Option<JsonValue>,
    correlation_id: Option<String>,
    party_id: Option<Uuid>,
) -> Result<Invoice, sqlx::Error> {
    sqlx::query_as::<_, Invoice>(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, due_at, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            correlation_id, party_id, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, NOW(), NOW())
        RETURNING
            id, app_id, tilled_invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, due_at, paid_at, hosted_url, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            correlation_id, party_id, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(tilled_invoice_id)
    .bind(ar_customer_id)
    .bind(subscription_id)
    .bind(status)
    .bind(amount_cents)
    .bind(currency)
    .bind(due_at)
    .bind(metadata)
    .bind(billing_period_start)
    .bind(billing_period_end)
    .bind(line_item_details)
    .bind(compliance_codes)
    .bind(correlation_id)
    .bind(party_id)
    .fetch_one(executor)
    .await
}

/// Update invoice fields. Returns the updated invoice (without party_id —
/// preserves existing response behavior).
pub async fn update_invoice_fields<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    status: &str,
    amount_cents: i32,
    due_at: Option<NaiveDateTime>,
    metadata: Option<JsonValue>,
) -> Result<Invoice, sqlx::Error> {
    sqlx::query_as::<_, Invoice>(
        r#"
        UPDATE ar_invoices
        SET status = $1, amount_cents = $2, due_at = $3, metadata = $4, updated_at = NOW()
        WHERE id = $5
        RETURNING
            id, app_id, tilled_invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, due_at, paid_at, hosted_url, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            correlation_id, created_at, updated_at
        "#,
    )
    .bind(status)
    .bind(amount_cents)
    .bind(due_at)
    .bind(metadata)
    .bind(id)
    .fetch_one(executor)
    .await
}

/// Set invoice status to 'open' with paid_at timestamp. Must be called within
/// a transaction. Returns invoice (without party_id — preserves existing
/// response behavior).
pub async fn set_invoice_finalized<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    paid_at: Option<NaiveDateTime>,
) -> Result<Invoice, sqlx::Error> {
    sqlx::query_as::<_, Invoice>(
        r#"
        UPDATE ar_invoices
        SET status = 'open', paid_at = $1, updated_at = NOW()
        WHERE id = $2
        RETURNING
            id, app_id, tilled_invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, due_at, paid_at, hosted_url, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            correlation_id, created_at, updated_at
        "#,
    )
    .bind(paid_at)
    .bind(id)
    .fetch_one(executor)
    .await
}
