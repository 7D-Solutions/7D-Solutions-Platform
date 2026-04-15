//! Payment method repository — all SQL operations for the payment methods domain.

use serde_json::Value as JsonValue;
use sqlx::PgExecutor;

use crate::models::PaymentMethod;

// ============================================================================
// Reads
// ============================================================================

/// Fetch a payment method by ID with tenant isolation (soft-delete filtered).
pub async fn fetch_with_tenant<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    app_id: &str,
) -> Result<Option<PaymentMethod>, sqlx::Error> {
    sqlx::query_as::<_, PaymentMethod>(
        r#"
        SELECT
            pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
            pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
            pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
            pm.deleted_at, pm.created_at, pm.updated_at
        FROM ar_payment_methods pm
        INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
        WHERE pm.id = $1 AND c.app_id = $2 AND pm.deleted_at IS NULL
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Find a payment method by its tilled_payment_method_id.
pub async fn find_by_tilled_id<'e>(
    executor: impl PgExecutor<'e>,
    tilled_pm_id: &str,
) -> Result<Option<PaymentMethod>, sqlx::Error> {
    sqlx::query_as::<_, PaymentMethod>(
        r#"
        SELECT
            id, app_id, ar_customer_id, tilled_payment_method_id,
            status, type, brand, last4, exp_month, exp_year,
            bank_name, bank_last4, is_default, metadata,
            deleted_at, created_at, updated_at
        FROM ar_payment_methods
        WHERE tilled_payment_method_id = $1
        "#,
    )
    .bind(tilled_pm_id)
    .fetch_optional(executor)
    .await
}

/// Count blocking charges (pending/authorized) for a customer.
pub async fn count_blocking_charges<'e>(
    executor: impl PgExecutor<'e>,
    customer_id: i32,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM ar_charges
        WHERE ar_customer_id = $1 AND status IN ('pending', 'authorized')
        "#,
    )
    .bind(customer_id)
    .fetch_one(executor)
    .await
}

/// Count payment methods matching filters.
pub async fn count_payment_methods<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: Option<i32>,
    status: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let mut sql = String::from(
        "SELECT COUNT(*) FROM ar_payment_methods pm \
         INNER JOIN ar_customers c ON pm.ar_customer_id = c.id \
         WHERE c.app_id = $1 AND pm.deleted_at IS NULL",
    );
    let mut bind_idx = 2;
    if customer_id.is_some() {
        sql.push_str(&format!(" AND pm.ar_customer_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if status.is_some() {
        sql.push_str(&format!(" AND pm.status = ${bind_idx}"));
    }

    let mut q = sqlx::query_scalar::<_, i64>(&sql).bind(app_id);
    if let Some(cid) = customer_id {
        q = q.bind(cid);
    }
    if let Some(st) = status {
        q = q.bind(st);
    }
    q.fetch_one(executor).await
}

/// List payment methods with optional filters and pagination.
pub async fn list_payment_methods<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: Option<i32>,
    status: Option<&str>,
    limit: i32,
    offset: i32,
) -> Result<Vec<PaymentMethod>, sqlx::Error> {
    let mut sql = String::from(
        r#"SELECT
            pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
            pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
            pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
            pm.deleted_at, pm.created_at, pm.updated_at
        FROM ar_payment_methods pm
        INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
        WHERE c.app_id = $1 AND pm.deleted_at IS NULL"#,
    );
    let mut idx = 2;
    if customer_id.is_some() {
        sql.push_str(&format!(" AND pm.ar_customer_id = ${idx}"));
        idx += 1;
    }
    if status.is_some() {
        sql.push_str(&format!(" AND pm.status = ${idx}"));
        idx += 1;
    }
    sql.push_str(&format!(
        " ORDER BY pm.is_default DESC, pm.created_at DESC LIMIT ${idx} OFFSET ${}",
        idx + 1
    ));

    let mut q = sqlx::query_as::<_, PaymentMethod>(&sql).bind(app_id);
    if let Some(cid) = customer_id {
        q = q.bind(cid);
    }
    if let Some(st) = status {
        q = q.bind(st);
    }
    q.bind(limit).bind(offset).fetch_all(executor).await
}

// ============================================================================
// Writes
// ============================================================================

/// Re-attach an existing payment method (update app_id, customer, status).
pub async fn reattach<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: i32,
    tilled_pm_id: &str,
) -> Result<PaymentMethod, sqlx::Error> {
    sqlx::query_as::<_, PaymentMethod>(
        r#"
        UPDATE ar_payment_methods
        SET app_id = $1, ar_customer_id = $2, status = 'pending_sync',
            deleted_at = NULL, updated_at = NOW()
        WHERE tilled_payment_method_id = $3
        RETURNING
            id, app_id, ar_customer_id, tilled_payment_method_id,
            status, type, brand, last4, exp_month, exp_year,
            bank_name, bank_last4, is_default, metadata,
            deleted_at, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(tilled_pm_id)
    .fetch_one(executor)
    .await
}

/// Insert a new payment method with pending_sync status.
pub async fn insert_pending<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: i32,
    tilled_pm_id: &str,
) -> Result<PaymentMethod, sqlx::Error> {
    sqlx::query_as::<_, PaymentMethod>(
        r#"
        INSERT INTO ar_payment_methods (
            app_id, ar_customer_id, tilled_payment_method_id,
            type, status, is_default, metadata, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'card', 'pending_sync', FALSE, '{}', NOW(), NOW())
        RETURNING
            id, app_id, ar_customer_id, tilled_payment_method_id,
            status, type, brand, last4, exp_month, exp_year,
            bank_name, bank_last4, is_default, metadata,
            deleted_at, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(tilled_pm_id)
    .fetch_one(executor)
    .await
}

/// Update payment method metadata.
pub async fn update_metadata<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    metadata: Option<JsonValue>,
) -> Result<PaymentMethod, sqlx::Error> {
    sqlx::query_as::<_, PaymentMethod>(
        r#"
        UPDATE ar_payment_methods
        SET metadata = $1, updated_at = NOW()
        WHERE id = $2
        RETURNING
            id, app_id, ar_customer_id, tilled_payment_method_id,
            status, type, brand, last4, exp_month, exp_year,
            bank_name, bank_last4, is_default, metadata,
            deleted_at, created_at, updated_at
        "#,
    )
    .bind(metadata)
    .bind(id)
    .fetch_one(executor)
    .await
}

/// Soft-delete a payment method.
pub async fn soft_delete<'e>(executor: impl PgExecutor<'e>, id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE ar_payment_methods
        SET deleted_at = NOW(), is_default = FALSE, updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Clear all default flags for a customer's payment methods.
pub async fn clear_default_flags<'e>(
    executor: impl PgExecutor<'e>,
    customer_id: i32,
    app_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE ar_payment_methods
        SET is_default = FALSE, updated_at = NOW()
        WHERE ar_customer_id = $1 AND app_id = $2
        "#,
    )
    .bind(customer_id)
    .bind(app_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Set a payment method as default and return the updated record.
pub async fn set_default_flag<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
) -> Result<PaymentMethod, sqlx::Error> {
    sqlx::query_as::<_, PaymentMethod>(
        r#"
        UPDATE ar_payment_methods
        SET is_default = TRUE, updated_at = NOW()
        WHERE id = $1
        RETURNING
            id, app_id, ar_customer_id, tilled_payment_method_id,
            status, type, brand, last4, exp_month, exp_year,
            bank_name, bank_last4, is_default, metadata,
            deleted_at, created_at, updated_at
        "#,
    )
    .bind(id)
    .fetch_one(executor)
    .await
}

// ============================================================================
// Webhook operations
// ============================================================================

/// Detach a payment method (webhook: soft-delete, idempotent).
pub async fn webhook_detach<'e>(
    executor: impl PgExecutor<'e>,
    tilled_pm_id: &str,
    app_id: &str,
) -> Result<(), sqlx::Error> {
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
    .execute(executor)
    .await?;
    Ok(())
}

/// Bind a pending_sync PM by its tilled_payment_method_id.
pub async fn bind_pending_by_tilled_id<'e>(
    executor: impl PgExecutor<'e>,
    tilled_pm_id: &str,
    brand: Option<&str>,
    last4: Option<&str>,
    exp_month: Option<i32>,
    exp_year: Option<i32>,
    pm_type: &str,
    data: &JsonValue,
    app_id: &str,
) -> Result<u64, sqlx::Error> {
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
    .bind(exp_month)
    .bind(exp_year)
    .bind(pm_type)
    .bind(data)
    .bind(app_id)
    .execute(executor)
    .await?;
    Ok(res.rows_affected())
}

/// Bind a pending_sync PM by customer (tilled_customer_id lookup).
pub async fn bind_pending_by_customer<'e>(
    executor: impl PgExecutor<'e>,
    tilled_pm_id: &str,
    brand: Option<&str>,
    last4: Option<&str>,
    exp_month: Option<i32>,
    exp_year: Option<i32>,
    pm_type: &str,
    data: &JsonValue,
    app_id: &str,
    tilled_customer_id: &str,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
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
    .bind(exp_month)
    .bind(exp_year)
    .bind(pm_type)
    .bind(data)
    .bind(app_id)
    .bind(tilled_customer_id)
    .execute(executor)
    .await?;
    Ok(res.rows_affected())
}

/// Update existing active PM with latest card details (webhook fallback).
pub async fn update_active_details<'e>(
    executor: impl PgExecutor<'e>,
    brand: Option<&str>,
    last4: Option<&str>,
    exp_month: Option<i32>,
    exp_year: Option<i32>,
    pm_type: &str,
    data: &JsonValue,
    tilled_pm_id: &str,
    app_id: &str,
) -> Result<(), sqlx::Error> {
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
    .bind(exp_month)
    .bind(exp_year)
    .bind(pm_type)
    .bind(data)
    .bind(tilled_pm_id)
    .bind(app_id)
    .execute(executor)
    .await?;
    Ok(())
}
