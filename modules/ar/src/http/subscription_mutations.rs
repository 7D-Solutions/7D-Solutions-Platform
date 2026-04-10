use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::domain::{customers, subscriptions};
use crate::models::{
    ApiError, CancelSubscriptionRequest, CreateSubscriptionRequest, Subscription,
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
    party_client_ext: Option<Extension<std::sync::Arc<platform_client_party::PartiesClient>>>,
    Json(req): Json<CreateSubscriptionRequest>,
) -> Result<(StatusCode, Json<Subscription>), ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    if req.plan_id.is_empty() || req.plan_name.is_empty() {
        return Err(ApiError::bad_request("Plan ID and name are required"));
    }

    if req.price_cents <= 0 {
        return Err(ApiError::bad_request("Price must be greater than 0"));
    }

    let _customer = customers::fetch_customer(&db, req.ar_customer_id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error fetching customer: {:?}", e);
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| {
            ApiError::not_found(format!("Customer {} not found", req.ar_customer_id))
        })?;

    if let Some(pid) = req.party_id {
        let Extension(verified) = claims.as_ref()
            .ok_or_else(|| ApiError::unauthorized("Missing authentication"))?;
        let Extension(party_client) = party_client_ext
            .ok_or_else(|| ApiError::new(503, "party_service_unavailable", "Party service not configured"))?;
        crate::integrations::party_client::verify_party(&party_client, pid, &app_id, verified)
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

    let subscription = subscriptions::insert_subscription(
        &db,
        &app_id,
        req.ar_customer_id,
        &req.plan_id,
        &req.plan_name,
        req.price_cents,
        SubscriptionStatus::PendingSync,
        &interval_unit,
        interval_count,
        now,
        current_period_end,
        &req.payment_method_id,
        req.metadata,
        req.party_id,
    )
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

    let existing = subscriptions::fetch_with_tenant(&db, id, &app_id)
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

    let subscription = subscriptions::update_fields(&db, id, &plan_id, &plan_name, price_cents, metadata)
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

    let _existing = subscriptions::fetch_with_tenant(&db, id, &app_id)
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
        subscriptions::set_cancel_at_period_end(&db, id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to schedule subscription cancellation: {:?}", e);
                ApiError::internal("Internal database error")
            })?
    } else {
        subscriptions::set_canceling(&db, id)
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
