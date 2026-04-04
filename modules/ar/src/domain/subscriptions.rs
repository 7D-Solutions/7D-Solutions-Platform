//! Subscription repository — all SQL operations for the subscriptions domain.

use chrono::NaiveDateTime;
use serde_json::Value as JsonValue;
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::models::{Subscription, SubscriptionInterval, SubscriptionStatus};

// ============================================================================
// Reads
// ============================================================================

/// Fetch a subscription by ID with tenant isolation.
pub async fn fetch_with_tenant<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    app_id: &str,
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
        INNER JOIN ar_customers c ON s.ar_customer_id = c.id
        WHERE s.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Count subscriptions matching filters.
pub async fn count_subscriptions<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: Option<i32>,
    status: Option<SubscriptionStatus>,
) -> Result<i64, sqlx::Error> {
    let mut sql = String::from(
        "SELECT COUNT(*) FROM ar_subscriptions s \
         INNER JOIN ar_customers c ON s.ar_customer_id = c.id \
         WHERE c.app_id = $1",
    );
    let mut bind_idx = 2;
    if customer_id.is_some() {
        sql.push_str(&format!(" AND s.ar_customer_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if status.is_some() {
        sql.push_str(&format!(" AND s.status = ${bind_idx}"));
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

/// List subscriptions with optional filters and pagination.
pub async fn list_subscriptions<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: Option<i32>,
    status: Option<SubscriptionStatus>,
    limit: i32,
    offset: i32,
) -> Result<Vec<Subscription>, sqlx::Error> {
    let mut sql = String::from(
        r#"SELECT
            s.id, s.app_id, s.ar_customer_id, s.tilled_subscription_id,
            s.plan_id, s.plan_name, s.price_cents, s.status, s.interval_unit, s.interval_count,
            s.billing_cycle_anchor, s.current_period_start, s.current_period_end,
            s.cancel_at_period_end, s.cancel_at, s.canceled_at, s.ended_at,
            s.payment_method_id, s.payment_method_type, s.metadata,
            s.update_source, s.updated_by, s.created_at, s.updated_at
        FROM ar_subscriptions s
        INNER JOIN ar_customers c ON s.ar_customer_id = c.id
        WHERE c.app_id = $1"#,
    );
    let mut idx = 2;
    if customer_id.is_some() {
        sql.push_str(&format!(" AND s.ar_customer_id = ${idx}"));
        idx += 1;
    }
    if status.is_some() {
        sql.push_str(&format!(" AND s.status = ${idx}"));
        idx += 1;
    }
    sql.push_str(&format!(
        " ORDER BY s.created_at DESC LIMIT ${idx} OFFSET ${}",
        idx + 1
    ));

    let mut q = sqlx::query_as::<_, Subscription>(&sql).bind(app_id);
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

/// Insert a new subscription.
#[allow(clippy::too_many_arguments)]
pub async fn insert_subscription<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: i32,
    plan_id: &str,
    plan_name: &str,
    price_cents: i32,
    status: SubscriptionStatus,
    interval_unit: &SubscriptionInterval,
    interval_count: i32,
    current_period_start: NaiveDateTime,
    current_period_end: NaiveDateTime,
    payment_method_id: &str,
    metadata: Option<JsonValue>,
    party_id: Option<Uuid>,
) -> Result<Subscription, sqlx::Error> {
    sqlx::query_as::<_, Subscription>(
        r#"
        INSERT INTO ar_subscriptions (
            app_id, ar_customer_id, tilled_subscription_id,
            plan_id, plan_name, price_cents, status, interval_unit, interval_count,
            current_period_start, current_period_end, cancel_at_period_end,
            payment_method_id, payment_method_type, metadata, party_id,
            created_at, updated_at
        )
        VALUES ($1, $2, NULL, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, NOW(), NOW())
        RETURNING
            id, app_id, ar_customer_id, tilled_subscription_id,
            plan_id, plan_name, price_cents, status, interval_unit, interval_count,
            billing_cycle_anchor, current_period_start, current_period_end,
            cancel_at_period_end, cancel_at, canceled_at, ended_at,
            payment_method_id, payment_method_type, metadata,
            update_source, updated_by, party_id, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(plan_id)
    .bind(plan_name)
    .bind(price_cents)
    .bind(status)
    .bind(interval_unit)
    .bind(interval_count)
    .bind(current_period_start)
    .bind(current_period_end)
    .bind(false)
    .bind(payment_method_id)
    .bind("card")
    .bind(metadata)
    .bind(party_id)
    .fetch_one(executor)
    .await
}

/// Update subscription plan/price/metadata fields.
pub async fn update_fields<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    plan_id: &str,
    plan_name: &str,
    price_cents: i32,
    metadata: Option<JsonValue>,
) -> Result<Subscription, sqlx::Error> {
    sqlx::query_as::<_, Subscription>(
        r#"
        UPDATE ar_subscriptions
        SET plan_id = $1, plan_name = $2, price_cents = $3, metadata = $4,
            update_source = 'local', updated_at = NOW()
        WHERE id = $5
        RETURNING
            id, app_id, ar_customer_id, tilled_subscription_id,
            plan_id, plan_name, price_cents, status, interval_unit, interval_count,
            billing_cycle_anchor, current_period_start, current_period_end,
            cancel_at_period_end, cancel_at, canceled_at, ended_at,
            payment_method_id, payment_method_type, metadata,
            update_source, updated_by, created_at, updated_at
        "#,
    )
    .bind(plan_id)
    .bind(plan_name)
    .bind(price_cents)
    .bind(metadata)
    .bind(id)
    .fetch_one(executor)
    .await
}

/// Set cancel_at_period_end = TRUE.
pub async fn set_cancel_at_period_end<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
) -> Result<Subscription, sqlx::Error> {
    sqlx::query_as::<_, Subscription>(
        r#"
        UPDATE ar_subscriptions
        SET cancel_at_period_end = TRUE, updated_at = NOW()
        WHERE id = $1
        RETURNING
            id, app_id, ar_customer_id, tilled_subscription_id,
            plan_id, plan_name, price_cents, status, interval_unit, interval_count,
            billing_cycle_anchor, current_period_start, current_period_end,
            cancel_at_period_end, cancel_at, canceled_at, ended_at,
            payment_method_id, payment_method_type, metadata,
            update_source, updated_by, created_at, updated_at
        "#,
    )
    .bind(id)
    .fetch_one(executor)
    .await
}

/// Set status to 'canceling' (immediate cancel).
pub async fn set_canceling<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
) -> Result<Subscription, sqlx::Error> {
    sqlx::query_as::<_, Subscription>(
        r#"
        UPDATE ar_subscriptions
        SET status = 'canceling', updated_at = NOW()
        WHERE id = $1
        RETURNING
            id, app_id, ar_customer_id, tilled_subscription_id,
            plan_id, plan_name, price_cents, status, interval_unit, interval_count,
            billing_cycle_anchor, current_period_start, current_period_end,
            cancel_at_period_end, cancel_at, canceled_at, ended_at,
            payment_method_id, payment_method_type, metadata,
            update_source, updated_by, created_at, updated_at
        "#,
    )
    .bind(id)
    .fetch_one(executor)
    .await
}

// ============================================================================
// Webhook operations
// ============================================================================

/// Bind a pending_sync subscription by customer (subscription.created webhook).
pub async fn bind_pending_by_customer<'e>(
    executor: impl PgExecutor<'e>,
    tilled_sub_id: &str,
    data: &JsonValue,
    period_start: Option<NaiveDateTime>,
    period_end: Option<NaiveDateTime>,
    app_id: &str,
    tilled_customer_id: &str,
) -> Result<u64, sqlx::Error> {
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
    .bind(data)
    .bind(period_start)
    .bind(period_end)
    .bind(app_id)
    .bind(tilled_customer_id)
    .execute(executor)
    .await?;
    Ok(res.rows_affected())
}

/// Update a subscription by tilled_subscription_id (created webhook fallback).
pub async fn update_by_tilled_id_created<'e>(
    executor: impl PgExecutor<'e>,
    data: &JsonValue,
    period_start: Option<NaiveDateTime>,
    period_end: Option<NaiveDateTime>,
    tilled_sub_id: &str,
    app_id: &str,
) -> Result<(), sqlx::Error> {
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
    .bind(data)
    .bind(period_start)
    .bind(period_end)
    .bind(tilled_sub_id)
    .bind(app_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Set subscription to canceled (terminal state, webhook).
pub async fn webhook_set_canceled<'e>(
    executor: impl PgExecutor<'e>,
    data: &JsonValue,
    tilled_sub_id: &str,
    app_id: &str,
) -> Result<(), sqlx::Error> {
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
    .bind(data)
    .bind(tilled_sub_id)
    .bind(app_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Update subscription via webhook (subscription.updated).
pub async fn webhook_update<'e>(
    executor: impl PgExecutor<'e>,
    status: &str,
    data: &JsonValue,
    period_start: Option<NaiveDateTime>,
    period_end: Option<NaiveDateTime>,
    tilled_sub_id: &str,
    app_id: &str,
) -> Result<(), sqlx::Error> {
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
    .bind(data)
    .bind(period_start)
    .bind(period_end)
    .bind(tilled_sub_id)
    .bind(app_id)
    .execute(executor)
    .await?;
    Ok(())
}
