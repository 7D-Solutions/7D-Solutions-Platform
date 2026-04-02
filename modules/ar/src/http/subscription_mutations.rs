use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{
    ApiError, CancelSubscriptionRequest, CreateSubscriptionRequest, Customer, Subscription,
    SubscriptionInterval, SubscriptionStatus, UpdateSubscriptionRequest,
};

/// POST /api/ar/subscriptions - Create a new subscription
#[utoipa::path(post, path = "/api/ar/subscriptions", tag = "Subscriptions",
    request_body = CreateSubscriptionRequest,
    responses(
        (status = 201, description = "Subscription created", body = Subscription),
        (status = 400, description = "Validation error", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn create_subscription(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateSubscriptionRequest>,
) -> Result<(StatusCode, Json<Subscription>), ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    if req.plan_id.is_empty() || req.plan_name.is_empty() {
        return Err(ApiError::bad_request("Plan ID and name are required"));
    }

    if req.price_cents <= 0 {
        return Err(ApiError::bad_request("Price must be greater than 0"));
    }

    let _customer = sqlx::query_as::<_, Customer>(
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
    .bind(req.ar_customer_id)
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching customer: {:?}", e);
        ApiError::internal("Internal database error")
    })?
    .ok_or_else(|| {
        ApiError::not_found(format!("Customer {} not found", req.ar_customer_id))
    })?;

    if let Some(pid) = req.party_id {
        let url = crate::integrations::party_client::party_master_url();
        let Extension(verified) = claims.as_ref()
            .ok_or_else(|| ApiError::unauthorized("Missing authentication"))?;
        crate::integrations::party_client::verify_party(&url, pid, &app_id, verified)
            .await
            .map_err(|e| {
                use crate::integrations::party_client::PartyClientError;
                tracing::warn!("Party validation failed for subscription create: {}", e);
                match &e {
                    PartyClientError::ServiceUnavailable(_) => {
                        ApiError::new(503, "party_service_unavailable", e.to_string())
                    }
                    _ => ApiError::new(422, "party_not_found", e.to_string()),
                }
            })?;
    }

    let interval_unit = req.interval_unit.unwrap_or(SubscriptionInterval::Month);
    let interval_count = req.interval_count.unwrap_or(1);

    let now = chrono::Utc::now().naive_utc();
    let current_period_end = match interval_unit {
        SubscriptionInterval::Day => now + chrono::Duration::days(interval_count as i64),
        SubscriptionInterval::Week => now + chrono::Duration::weeks(interval_count as i64),
        SubscriptionInterval::Month => now + chrono::Duration::days(30 * interval_count as i64),
        SubscriptionInterval::Year => now + chrono::Duration::days(365 * interval_count as i64),
    };

    let subscription = sqlx::query_as::<_, Subscription>(
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
    .bind(&app_id)
    .bind(req.ar_customer_id)
    .bind(&req.plan_id)
    .bind(&req.plan_name)
    .bind(req.price_cents)
    .bind(SubscriptionStatus::PendingSync)
    .bind(&interval_unit)
    .bind(interval_count)
    .bind(now)
    .bind(current_period_end)
    .bind(false)
    .bind(&req.payment_method_id)
    .bind("card")
    .bind(req.metadata)
    .bind(req.party_id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create subscription: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    tracing::info!(
        "Created subscription {} for customer {}",
        subscription.id,
        req.ar_customer_id
    );

    Ok((StatusCode::CREATED, Json(subscription)))
}

/// PUT /api/ar/subscriptions/:id - Update subscription
#[utoipa::path(put, path = "/api/ar/subscriptions/{id}", tag = "Subscriptions",
    params(("id" = i32, Path, description = "Subscription ID")),
    request_body = UpdateSubscriptionRequest,
    responses(
        (status = 200, description = "Subscription updated", body = Subscription),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn update_subscription(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<UpdateSubscriptionRequest>,
) -> Result<Json<Subscription>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let existing = sqlx::query_as::<_, Subscription>(
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

    if req.plan_id.is_none()
        && req.plan_name.is_none()
        && req.price_cents.is_none()
        && req.metadata.is_none()
    {
        return Err(ApiError::bad_request("No valid fields to update"));
    }

    let plan_id = req.plan_id.unwrap_or(existing.plan_id);
    let plan_name = req.plan_name.unwrap_or(existing.plan_name);
    let price_cents = req.price_cents.unwrap_or(existing.price_cents);
    let metadata = req.metadata.or(existing.metadata);

    let subscription = sqlx::query_as::<_, Subscription>(
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
    .bind(&plan_id)
    .bind(&plan_name)
    .bind(price_cents)
    .bind(metadata)
    .bind(id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update subscription: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    tracing::info!("Updated subscription {}", id);

    Ok(Json(subscription))
}

/// POST /api/ar/subscriptions/:id/cancel - Cancel subscription
#[utoipa::path(post, path = "/api/ar/subscriptions/{id}/cancel", tag = "Subscriptions",
    params(("id" = i32, Path, description = "Subscription ID")),
    request_body = CancelSubscriptionRequest,
    responses(
        (status = 200, description = "Subscription canceled", body = Subscription),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn cancel_subscription(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<CancelSubscriptionRequest>,
) -> Result<Json<Subscription>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let _existing = sqlx::query_as::<_, Subscription>(
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

    let cancel_at_period_end = req.cancel_at_period_end.unwrap_or(false);

    let subscription = if cancel_at_period_end {
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
        .fetch_one(&db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to schedule subscription cancellation: {:?}", e);
            ApiError::internal("Internal database error")
        })?
    } else {
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
        .fetch_one(&db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to cancel subscription: {:?}", e);
            ApiError::internal("Internal database error")
        })?
    };

    tracing::info!(
        "Canceled subscription {} (at_period_end={})",
        id,
        cancel_at_period_end
    );

    Ok(Json(subscription))
}
