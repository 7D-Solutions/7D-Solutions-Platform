//! Customer repository — all SQL operations for the customers domain.

use serde_json::Value as JsonValue;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::models::Customer;

// ============================================================================
// Reads
// ============================================================================

/// Fetch a customer by ID and app_id. Used as a guard across many handlers.
pub async fn fetch_customer<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
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
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Fetch a customer with party_id included (for endpoints that return it).
pub async fn fetch_customer_with_party<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
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
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Check if a customer with the given email already exists for this app.
pub async fn check_email_exists<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    email: &str,
) -> Result<Option<(i32,)>, sqlx::Error> {
    sqlx::query_as("SELECT id FROM ar_customers WHERE app_id = $1 AND email = $2 LIMIT 1")
        .bind(app_id)
        .bind(email)
        .fetch_optional(executor)
        .await
}

/// Count customers matching the optional external_customer_id filter.
pub async fn count_customers<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    external_customer_id: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let mut sql = String::from("SELECT COUNT(*) FROM ar_customers WHERE app_id = $1");
    if external_customer_id.is_some() {
        sql.push_str(" AND external_customer_id = $2");
    }
    let mut q = sqlx::query_scalar::<_, i64>(&sql).bind(app_id);
    if let Some(ext_id) = external_customer_id {
        q = q.bind(ext_id);
    }
    q.fetch_one(executor).await
}

/// List customers with optional external_customer_id filter and pagination.
pub async fn list_customers<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    external_customer_id: Option<String>,
    limit: i32,
    offset: i32,
) -> Result<Vec<Customer>, sqlx::Error> {
    if let Some(external_id) = external_customer_id {
        sqlx::query_as::<_, Customer>(
            r#"
            SELECT
                id, app_id, external_customer_id, tilled_customer_id, status,
                email, name, default_payment_method_id, payment_method_type,
                metadata, update_source, updated_by, delinquent_since,
                grace_period_end, next_retry_at, retry_attempt_count,
                created_at, updated_at
            FROM ar_customers
            WHERE app_id = $1 AND external_customer_id = $2
            ORDER BY created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(app_id)
        .bind(external_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(executor)
        .await
    } else {
        sqlx::query_as::<_, Customer>(
            r#"
            SELECT
                id, app_id, external_customer_id, tilled_customer_id, status,
                email, name, default_payment_method_id, payment_method_type,
                metadata, update_source, updated_by, delinquent_since,
                grace_period_end, next_retry_at, retry_attempt_count,
                created_at, updated_at
            FROM ar_customers
            WHERE app_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(app_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(executor)
        .await
    }
}

// ============================================================================
// Writes
// ============================================================================

/// Insert a new customer. Returns the created customer.
pub async fn insert_customer<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    external_customer_id: Option<String>,
    email: &str,
    name: Option<String>,
    metadata: Option<JsonValue>,
    party_id: Option<Uuid>,
) -> Result<Customer, sqlx::Error> {
    sqlx::query_as::<_, Customer>(
        r#"
        INSERT INTO ar_customers (
            app_id, external_customer_id, email, name, metadata,
            status, tilled_customer_id, retry_attempt_count, party_id, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, 'pending_sync', NULL, 0, $6, NOW(), NOW())
        RETURNING
            id, app_id, external_customer_id, tilled_customer_id, status,
            email, name, default_payment_method_id, payment_method_type,
            metadata, update_source, updated_by, delinquent_since,
            grace_period_end, next_retry_at, retry_attempt_count,
            party_id, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(external_customer_id)
    .bind(email)
    .bind(name)
    .bind(metadata)
    .bind(party_id)
    .fetch_one(executor)
    .await
}

/// Update customer fields. Returns the updated customer (with party_id).
pub async fn update_customer<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    email: &str,
    name: Option<String>,
    metadata: Option<JsonValue>,
    party_id: Option<Uuid>,
) -> Result<Customer, sqlx::Error> {
    sqlx::query_as::<_, Customer>(
        r#"
        UPDATE ar_customers
        SET email = $1, name = $2, metadata = $3, party_id = $4,
            update_source = 'local', updated_at = NOW()
        WHERE id = $5
        RETURNING
            id, app_id, external_customer_id, tilled_customer_id, status,
            email, name, default_payment_method_id, payment_method_type,
            metadata, update_source, updated_by, delinquent_since,
            grace_period_end, next_retry_at, retry_attempt_count,
            party_id, created_at, updated_at
        "#,
    )
    .bind(email)
    .bind(name)
    .bind(metadata)
    .bind(party_id)
    .bind(id)
    .fetch_one(executor)
    .await
}

/// Clear the default payment method on a customer.
pub async fn clear_default_payment_method<'e>(
    executor: impl PgExecutor<'e>,
    customer_id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE ar_customers
        SET default_payment_method_id = NULL, payment_method_type = NULL, updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(customer_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Set the default payment method on a customer.
pub async fn set_default_payment_method<'e>(
    executor: impl PgExecutor<'e>,
    customer_id: i32,
    tilled_payment_method_id: &str,
    payment_type: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE ar_customers
        SET default_payment_method_id = $1, payment_method_type = $2, updated_at = NOW()
        WHERE id = $3
        "#,
    )
    .bind(tilled_payment_method_id)
    .bind(payment_type)
    .bind(customer_id)
    .execute(executor)
    .await?;
    Ok(())
}

// ============================================================================
// Webhook operations
// ============================================================================

/// Bind a pending_sync customer by email (provider ID not yet set).
pub async fn bind_pending_by_email<'e>(
    executor: impl PgExecutor<'e>,
    tilled_customer_id: &str,
    name: Option<&str>,
    data: &JsonValue,
    app_id: &str,
    email: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
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
    .bind(data)
    .bind(app_id)
    .bind(email)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Upsert a customer by tilled_customer_id (globally unique).
pub async fn upsert_by_tilled_id<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    tilled_customer_id: &str,
    email: &str,
    name: Option<&str>,
    status: &str,
    data: &JsonValue,
) -> Result<(), sqlx::Error> {
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
    .bind(status)
    .bind(data)
    .execute(executor)
    .await?;
    Ok(())
}
