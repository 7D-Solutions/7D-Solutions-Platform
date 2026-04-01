use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{
    ApiError, CancelSubscriptionRequest, CreateSubscriptionRequest, Customer,
    ListSubscriptionsQuery, Subscription, SubscriptionInterval, SubscriptionStatus,
    UpdateSubscriptionRequest,
};

/// POST /api/ar/subscriptions - Create a new subscription
pub async fn create_subscription(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateSubscriptionRequest>,
) -> Result<(StatusCode, Json<Subscription>), ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    // Validate required fields
    if req.plan_id.is_empty() || req.plan_name.is_empty() {
        return Err(ApiError::bad_request("Plan ID and name are required"));
    }

    if req.price_cents <= 0 {
        return Err(ApiError::bad_request("Price must be greater than 0"));
    }

    // Verify customer exists and belongs to app
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

    // Validate party_id exists in Party Master if provided
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

    // Set defaults
    let interval_unit = req.interval_unit.unwrap_or(SubscriptionInterval::Month);
    let interval_count = req.interval_count.unwrap_or(1);

    // Calculate period dates (simplified — in production the webhook response refines these)
    let now = chrono::Utc::now().naive_utc();
    let current_period_end = match interval_unit {
        SubscriptionInterval::Day => now + chrono::Duration::days(interval_count as i64),
        SubscriptionInterval::Week => now + chrono::Duration::weeks(interval_count as i64),
        SubscriptionInterval::Month => now + chrono::Duration::days(30 * interval_count as i64),
        SubscriptionInterval::Year => now + chrono::Duration::days(365 * interval_count as i64),
    };

    // Create subscription in database (local-first, low-code pattern).
    // Status starts as pending_sync with NULL tilled_subscription_id.
    // The provider webhook (subscription.created) will set the real provider ID
    // and transition status to active.
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
    .bind(false) // cancel_at_period_end
    .bind(&req.payment_method_id)
    .bind("card") // Default payment method type
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

/// GET /api/ar/subscriptions/:id - Get subscription by ID
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
pub async fn list_subscriptions(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListSubscriptionsQuery>,
) -> Result<Json<Vec<Subscription>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100); // Max 100 per page
    let offset = query.offset.unwrap_or(0).max(0);

    // Build query based on filters
    let subscriptions = match (query.customer_id, query.status) {
        (Some(customer_id), Some(status)) => {
            // Filter by both customer and status
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
                WHERE c.app_id = $1 AND s.ar_customer_id = $2 AND s.status = $3
                ORDER BY s.created_at DESC
                LIMIT $4 OFFSET $5
                "#,
            )
            .bind(&app_id)
            .bind(customer_id)
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (Some(customer_id), None) => {
            // Filter by customer only
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
                WHERE c.app_id = $1 AND s.ar_customer_id = $2
                ORDER BY s.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(&app_id)
            .bind(customer_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (None, Some(status)) => {
            // Filter by status only
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
                WHERE c.app_id = $1 AND s.status = $2
                ORDER BY s.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(&app_id)
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (None, None) => {
            // List all subscriptions for app
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
                WHERE c.app_id = $1
                ORDER BY s.created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(&app_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
    }
    .map_err(|e| {
        tracing::error!("Database error listing subscriptions: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    Ok(Json(subscriptions))
}

/// PUT /api/ar/subscriptions/:id - Update subscription
pub async fn update_subscription(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<UpdateSubscriptionRequest>,
) -> Result<Json<Subscription>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    // Verify subscription exists and belongs to app
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

    // Validate at least one field is being updated
    if req.plan_id.is_none()
        && req.plan_name.is_none()
        && req.price_cents.is_none()
        && req.metadata.is_none()
    {
        return Err(ApiError::bad_request("No valid fields to update"));
    }

    // Build update based on provided fields
    let plan_id = req.plan_id.unwrap_or(existing.plan_id);
    let plan_name = req.plan_name.unwrap_or(existing.plan_name);
    let price_cents = req.price_cents.unwrap_or(existing.price_cents);
    let metadata = req.metadata.or(existing.metadata);

    // Update local fields immediately. Set update_source = 'local' to distinguish
    // from webhook-driven updates.
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
pub async fn cancel_subscription(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<CancelSubscriptionRequest>,
) -> Result<Json<Subscription>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    // Verify subscription exists and belongs to app
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
        // Schedule cancellation at period end
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
        // Immediate cancellation intent. Status transitions to 'canceling' —
        // the provider webhook (subscription.canceled) will finalize to 'canceled'.
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
