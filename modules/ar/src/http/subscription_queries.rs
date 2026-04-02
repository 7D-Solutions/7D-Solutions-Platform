use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{ApiError, ListSubscriptionsQuery, PaginatedResponse, Subscription};

/// GET /api/ar/subscriptions/:id - Get subscription by ID
#[utoipa::path(get, path = "/api/ar/subscriptions/{id}", tag = "Subscriptions",
    params(("id" = i32, Path, description = "Subscription ID")),
    responses(
        (status = 200, description = "Subscription found", body = Subscription),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_subscription(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Subscription>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let subscription = sqlx::query_as::<_, Subscription>(
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
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching subscription: {:?}", e);
        ApiError::internal("Internal database error")
    })?
    .ok_or_else(|| {
        ApiError::not_found(format!("Subscription {} not found", id))
    })?;

    Ok(Json(subscription))
}

/// GET /api/ar/subscriptions - List subscriptions (with optional filtering)
#[utoipa::path(get, path = "/api/ar/subscriptions", tag = "Subscriptions",
    params(ListSubscriptionsQuery),
    responses(
        (status = 200, description = "Paginated subscriptions", body = PaginatedResponse<Subscription>),
    ),
    security(("bearer" = [])))]
pub async fn list_subscriptions(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListSubscriptionsQuery>,
) -> Result<Json<PaginatedResponse<Subscription>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Count total matching rows
    let mut count_sql = String::from(
        "SELECT COUNT(*) FROM ar_subscriptions s \
         INNER JOIN ar_customers c ON s.ar_customer_id = c.id \
         WHERE c.app_id = $1",
    );
    let mut bind_idx = 2;
    if query.customer_id.is_some() {
        count_sql.push_str(&format!(" AND s.ar_customer_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if query.status.is_some() {
        count_sql.push_str(&format!(" AND s.status = ${bind_idx}"));
    }

    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(&app_id);
    if let Some(cid) = query.customer_id {
        count_q = count_q.bind(cid);
    }
    if let Some(ref st) = query.status {
        count_q = count_q.bind(st.clone());
    }
    let total_items = count_q.fetch_one(&db).await.map_err(|e| {
        tracing::error!("Database error counting subscriptions: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    // Fetch data
    let mut data_sql = String::from(
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
    let mut data_idx = 2;
    if query.customer_id.is_some() {
        data_sql.push_str(&format!(" AND s.ar_customer_id = ${data_idx}"));
        data_idx += 1;
    }
    if query.status.is_some() {
        data_sql.push_str(&format!(" AND s.status = ${data_idx}"));
        data_idx += 1;
    }
    data_sql.push_str(&format!(
        " ORDER BY s.created_at DESC LIMIT ${data_idx} OFFSET ${}",
        data_idx + 1
    ));

    let mut data_q = sqlx::query_as::<_, Subscription>(&data_sql).bind(&app_id);
    if let Some(cid) = query.customer_id {
        data_q = data_q.bind(cid);
    }
    if let Some(ref st) = query.status {
        data_q = data_q.bind(st.clone());
    }
    let subscriptions = data_q
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
        .await
        .map_err(|e| {
            tracing::error!("Database error listing subscriptions: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(
        subscriptions,
        page,
        limit as i64,
        total_items,
    )))
}
