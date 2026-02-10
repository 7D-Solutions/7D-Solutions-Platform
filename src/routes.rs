use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use sqlx::PgPool;

use crate::models::{
    CancelSubscriptionRequest, CreateCustomerRequest, CreateSubscriptionRequest, Customer,
    ErrorResponse, ListCustomersQuery, ListSubscriptionsQuery, Subscription,
    SubscriptionInterval, SubscriptionStatus, UpdateCustomerRequest,
    UpdateSubscriptionRequest,
};

pub fn ar_router(db: PgPool) -> Router {
    Router::new()
        // Customer endpoints
        .route("/api/ar/customers", post(create_customer).get(list_customers))
        .route(
            "/api/ar/customers/{id}",
            get(get_customer).put(update_customer),
        )
        // Subscription endpoints
        .route(
            "/api/ar/subscriptions",
            post(create_subscription).get(list_subscriptions),
        )
        .route(
            "/api/ar/subscriptions/{id}",
            get(get_subscription).put(update_subscription),
        )
        .route(
            "/api/ar/subscriptions/{id}/cancel",
            post(cancel_subscription),
        )
        .with_state(db)
}

/// POST /api/ar/customers - Create a new customer
async fn create_customer(
    State(db): State<PgPool>,
    Json(req): Json<CreateCustomerRequest>,
) -> Result<(StatusCode, Json<Customer>), (StatusCode, Json<ErrorResponse>)> {
    // Validate email
    if req.email.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("validation_error", "Email is required")),
        ));
    }

    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    // Create customer in database (local-first pattern)
    let customer = sqlx::query_as::<_, Customer>(
        r#"
        INSERT INTO billing_customers (
            app_id, external_customer_id, email, name, metadata,
            status, tilled_customer_id, retry_attempt_count, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, 'pending', NULL, 0, NOW(), NOW())
        RETURNING
            id, app_id, external_customer_id, tilled_customer_id, status,
            email, name, default_payment_method_id, payment_method_type,
            metadata, update_source, updated_by, delinquent_since,
            grace_period_end, next_retry_at, retry_attempt_count,
            created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(req.external_customer_id)
    .bind(&req.email)
    .bind(req.name)
    .bind(req.metadata)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create customer: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to create customer: {}", e),
            )),
        )
    })?;

    // TODO: Integrate with Tilled API to create remote customer
    // For now, we'll update status to 'active' immediately
    let customer = sqlx::query_as::<_, Customer>(
        r#"
        UPDATE billing_customers
        SET status = 'active', updated_at = NOW()
        WHERE id = $1
        RETURNING
            id, app_id, external_customer_id, tilled_customer_id, status,
            email, name, default_payment_method_id, payment_method_type,
            metadata, update_source, updated_by, delinquent_since,
            grace_period_end, next_retry_at, retry_attempt_count,
            created_at, updated_at
        "#,
    )
    .bind(customer.id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update customer status: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to update customer: {}", e),
            )),
        )
    })?;

    Ok((StatusCode::CREATED, Json(customer)))
}

/// GET /api/ar/customers/:id - Get customer by ID
async fn get_customer(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<Customer>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    let customer = sqlx::query_as::<_, Customer>(
        r#"
        SELECT
            id, app_id, external_customer_id, tilled_customer_id, status,
            email, name, default_payment_method_id, payment_method_type,
            metadata, update_source, updated_by, delinquent_since,
            grace_period_end, next_retry_at, retry_attempt_count,
            created_at, updated_at
        FROM billing_customers
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching customer: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch customer: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Customer {} not found", id),
            )),
        )
    })?;

    Ok(Json(customer))
}

/// GET /api/ar/customers - List customers (with optional filtering)
async fn list_customers(
    State(db): State<PgPool>,
    Query(query): Query<ListCustomersQuery>,
) -> Result<Json<Vec<Customer>>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    let limit = query.limit.unwrap_or(50).min(100); // Max 100 per page
    let offset = query.offset.unwrap_or(0).max(0);

    let customers = if let Some(external_id) = query.external_customer_id {
        // Filter by external_customer_id
        sqlx::query_as::<_, Customer>(
            r#"
            SELECT
                id, app_id, external_customer_id, tilled_customer_id, status,
                email, name, default_payment_method_id, payment_method_type,
                metadata, update_source, updated_by, delinquent_since,
                grace_period_end, next_retry_at, retry_attempt_count,
                created_at, updated_at
            FROM billing_customers
            WHERE app_id = $1 AND external_customer_id = $2
            ORDER BY created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(app_id)
        .bind(external_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
        .await
    } else {
        // List all customers for app
        sqlx::query_as::<_, Customer>(
            r#"
            SELECT
                id, app_id, external_customer_id, tilled_customer_id, status,
                email, name, default_payment_method_id, payment_method_type,
                metadata, update_source, updated_by, delinquent_since,
                grace_period_end, next_retry_at, retry_attempt_count,
                created_at, updated_at
            FROM billing_customers
            WHERE app_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(app_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
        .await
    }
    .map_err(|e| {
        tracing::error!("Database error listing customers: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to list customers: {}", e),
            )),
        )
    })?;

    Ok(Json(customers))
}

/// PUT /api/ar/customers/:id - Update customer
async fn update_customer(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
    Json(req): Json<UpdateCustomerRequest>,
) -> Result<Json<Customer>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    // Verify customer exists and belongs to app
    let existing = sqlx::query_as::<_, Customer>(
        r#"
        SELECT
            id, app_id, external_customer_id, tilled_customer_id, status,
            email, name, default_payment_method_id, payment_method_type,
            metadata, update_source, updated_by, delinquent_since,
            grace_period_end, next_retry_at, retry_attempt_count,
            created_at, updated_at
        FROM billing_customers
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching customer: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch customer: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Customer {} not found", id),
            )),
        )
    })?;

    // Validate at least one field is being updated
    if req.email.is_none() && req.name.is_none() && req.metadata.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "No valid fields to update",
            )),
        ));
    }

    // Build dynamic update query based on provided fields
    let email = req.email.unwrap_or(existing.email);
    let name = req.name.or(existing.name);
    let metadata = req.metadata.or(existing.metadata);

    let customer = sqlx::query_as::<_, Customer>(
        r#"
        UPDATE billing_customers
        SET email = $1, name = $2, metadata = $3, updated_at = NOW()
        WHERE id = $4
        RETURNING
            id, app_id, external_customer_id, tilled_customer_id, status,
            email, name, default_payment_method_id, payment_method_type,
            metadata, update_source, updated_by, delinquent_since,
            grace_period_end, next_retry_at, retry_attempt_count,
            created_at, updated_at
        "#,
    )
    .bind(&email)
    .bind(name)
    .bind(metadata)
    .bind(id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update customer: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to update customer: {}", e),
            )),
        )
    })?;

    // TODO: Sync with Tilled API

    Ok(Json(customer))
}

// ============================================================================
// SUBSCRIPTION ENDPOINTS
// ============================================================================

/// POST /api/ar/subscriptions - Create a new subscription
async fn create_subscription(
    State(db): State<PgPool>,
    Json(req): Json<CreateSubscriptionRequest>,
) -> Result<(StatusCode, Json<Subscription>), (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    // Validate required fields
    if req.plan_id.is_empty() || req.plan_name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "Plan ID and name are required",
            )),
        ));
    }

    if req.price_cents <= 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "Price must be greater than 0",
            )),
        ));
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
        FROM billing_customers
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(req.billing_customer_id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching customer: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch customer: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Customer {} not found", req.billing_customer_id),
            )),
        )
    })?;

    // Generate a placeholder Tilled subscription ID
    // TODO: Integrate with Tilled API to create actual subscription
    let tilled_subscription_id = format!("sub_{}", uuid::Uuid::new_v4());

    // Set defaults
    let interval_unit = req.interval_unit.unwrap_or(SubscriptionInterval::Month);
    let interval_count = req.interval_count.unwrap_or(1);

    // Calculate period dates (simplified - in production would use Tilled response)
    let now = chrono::Utc::now().naive_utc();
    let current_period_end = match interval_unit {
        SubscriptionInterval::Day => now + chrono::Duration::days(interval_count as i64),
        SubscriptionInterval::Week => now + chrono::Duration::weeks(interval_count as i64),
        SubscriptionInterval::Month => {
            now + chrono::Duration::days(30 * interval_count as i64)
        }
        SubscriptionInterval::Year => now + chrono::Duration::days(365 * interval_count as i64),
    };

    // Create subscription in database
    let subscription = sqlx::query_as::<_, Subscription>(
        r#"
        INSERT INTO billing_subscriptions (
            app_id, billing_customer_id, tilled_subscription_id,
            plan_id, plan_name, price_cents, status, interval_unit, interval_count,
            current_period_start, current_period_end, cancel_at_period_end,
            payment_method_id, payment_method_type, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, NOW(), NOW())
        RETURNING
            id, app_id, billing_customer_id, tilled_subscription_id,
            plan_id, plan_name, price_cents, status, interval_unit, interval_count,
            billing_cycle_anchor, current_period_start, current_period_end,
            cancel_at_period_end, cancel_at, canceled_at, ended_at,
            payment_method_id, payment_method_type, metadata,
            update_source, updated_by, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(req.billing_customer_id)
    .bind(&tilled_subscription_id)
    .bind(&req.plan_id)
    .bind(&req.plan_name)
    .bind(req.price_cents)
    .bind(SubscriptionStatus::Active) // Default to active for now
    .bind(&interval_unit)
    .bind(interval_count)
    .bind(&now)
    .bind(&current_period_end)
    .bind(false) // cancel_at_period_end
    .bind(&req.payment_method_id)
    .bind("card") // Default payment method type
    .bind(req.metadata)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create subscription: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to create subscription: {}", e),
            )),
        )
    })?;

    tracing::info!(
        "Created subscription {} for customer {}",
        subscription.id,
        req.billing_customer_id
    );

    Ok((StatusCode::CREATED, Json(subscription)))
}

/// GET /api/ar/subscriptions/:id - Get subscription by ID
async fn get_subscription(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<Subscription>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    let subscription = sqlx::query_as::<_, Subscription>(
        r#"
        SELECT
            s.id, s.app_id, s.billing_customer_id, s.tilled_subscription_id,
            s.plan_id, s.plan_name, s.price_cents, s.status, s.interval_unit, s.interval_count,
            s.billing_cycle_anchor, s.current_period_start, s.current_period_end,
            s.cancel_at_period_end, s.cancel_at, s.canceled_at, s.ended_at,
            s.payment_method_id, s.payment_method_type, s.metadata,
            s.update_source, s.updated_by, s.created_at, s.updated_at
        FROM billing_subscriptions s
        INNER JOIN billing_customers c ON s.billing_customer_id = c.id
        WHERE s.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching subscription: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch subscription: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Subscription {} not found", id),
            )),
        )
    })?;

    Ok(Json(subscription))
}

/// GET /api/ar/subscriptions - List subscriptions (with optional filtering)
async fn list_subscriptions(
    State(db): State<PgPool>,
    Query(query): Query<ListSubscriptionsQuery>,
) -> Result<Json<Vec<Subscription>>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    let limit = query.limit.unwrap_or(50).min(100); // Max 100 per page
    let offset = query.offset.unwrap_or(0).max(0);

    // Build query based on filters
    let subscriptions = match (query.customer_id, query.status) {
        (Some(customer_id), Some(status)) => {
            // Filter by both customer and status
            sqlx::query_as::<_, Subscription>(
                r#"
                SELECT
                    s.id, s.app_id, s.billing_customer_id, s.tilled_subscription_id,
                    s.plan_id, s.plan_name, s.price_cents, s.status, s.interval_unit, s.interval_count,
                    s.billing_cycle_anchor, s.current_period_start, s.current_period_end,
                    s.cancel_at_period_end, s.cancel_at, s.canceled_at, s.ended_at,
                    s.payment_method_id, s.payment_method_type, s.metadata,
                    s.update_source, s.updated_by, s.created_at, s.updated_at
                FROM billing_subscriptions s
                INNER JOIN billing_customers c ON s.billing_customer_id = c.id
                WHERE c.app_id = $1 AND s.billing_customer_id = $2 AND s.status = $3
                ORDER BY s.created_at DESC
                LIMIT $4 OFFSET $5
                "#,
            )
            .bind(app_id)
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
                    s.id, s.app_id, s.billing_customer_id, s.tilled_subscription_id,
                    s.plan_id, s.plan_name, s.price_cents, s.status, s.interval_unit, s.interval_count,
                    s.billing_cycle_anchor, s.current_period_start, s.current_period_end,
                    s.cancel_at_period_end, s.cancel_at, s.canceled_at, s.ended_at,
                    s.payment_method_id, s.payment_method_type, s.metadata,
                    s.update_source, s.updated_by, s.created_at, s.updated_at
                FROM billing_subscriptions s
                INNER JOIN billing_customers c ON s.billing_customer_id = c.id
                WHERE c.app_id = $1 AND s.billing_customer_id = $2
                ORDER BY s.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(app_id)
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
                    s.id, s.app_id, s.billing_customer_id, s.tilled_subscription_id,
                    s.plan_id, s.plan_name, s.price_cents, s.status, s.interval_unit, s.interval_count,
                    s.billing_cycle_anchor, s.current_period_start, s.current_period_end,
                    s.cancel_at_period_end, s.cancel_at, s.canceled_at, s.ended_at,
                    s.payment_method_id, s.payment_method_type, s.metadata,
                    s.update_source, s.updated_by, s.created_at, s.updated_at
                FROM billing_subscriptions s
                INNER JOIN billing_customers c ON s.billing_customer_id = c.id
                WHERE c.app_id = $1 AND s.status = $2
                ORDER BY s.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(app_id)
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
                    s.id, s.app_id, s.billing_customer_id, s.tilled_subscription_id,
                    s.plan_id, s.plan_name, s.price_cents, s.status, s.interval_unit, s.interval_count,
                    s.billing_cycle_anchor, s.current_period_start, s.current_period_end,
                    s.cancel_at_period_end, s.cancel_at, s.canceled_at, s.ended_at,
                    s.payment_method_id, s.payment_method_type, s.metadata,
                    s.update_source, s.updated_by, s.created_at, s.updated_at
                FROM billing_subscriptions s
                INNER JOIN billing_customers c ON s.billing_customer_id = c.id
                WHERE c.app_id = $1
                ORDER BY s.created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(app_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
    }
    .map_err(|e| {
        tracing::error!("Database error listing subscriptions: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to list subscriptions: {}", e),
            )),
        )
    })?;

    Ok(Json(subscriptions))
}

/// PUT /api/ar/subscriptions/:id - Update subscription
async fn update_subscription(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
    Json(req): Json<UpdateSubscriptionRequest>,
) -> Result<Json<Subscription>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    // Verify subscription exists and belongs to app
    let existing = sqlx::query_as::<_, Subscription>(
        r#"
        SELECT
            s.id, s.app_id, s.billing_customer_id, s.tilled_subscription_id,
            s.plan_id, s.plan_name, s.price_cents, s.status, s.interval_unit, s.interval_count,
            s.billing_cycle_anchor, s.current_period_start, s.current_period_end,
            s.cancel_at_period_end, s.cancel_at, s.canceled_at, s.ended_at,
            s.payment_method_id, s.payment_method_type, s.metadata,
            s.update_source, s.updated_by, s.created_at, s.updated_at
        FROM billing_subscriptions s
        INNER JOIN billing_customers c ON s.billing_customer_id = c.id
        WHERE s.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching subscription: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch subscription: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Subscription {} not found", id),
            )),
        )
    })?;

    // Validate at least one field is being updated
    if req.plan_id.is_none()
        && req.plan_name.is_none()
        && req.price_cents.is_none()
        && req.metadata.is_none()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "No valid fields to update",
            )),
        ));
    }

    // Build update based on provided fields
    let plan_id = req.plan_id.unwrap_or(existing.plan_id);
    let plan_name = req.plan_name.unwrap_or(existing.plan_name);
    let price_cents = req.price_cents.unwrap_or(existing.price_cents);
    let metadata = req.metadata.or(existing.metadata);

    let subscription = sqlx::query_as::<_, Subscription>(
        r#"
        UPDATE billing_subscriptions
        SET plan_id = $1, plan_name = $2, price_cents = $3, metadata = $4, updated_at = NOW()
        WHERE id = $5
        RETURNING
            id, app_id, billing_customer_id, tilled_subscription_id,
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to update subscription: {}", e),
            )),
        )
    })?;

    tracing::info!("Updated subscription {}", id);

    // TODO: Sync with Tilled API

    Ok(Json(subscription))
}

/// POST /api/ar/subscriptions/:id/cancel - Cancel subscription
async fn cancel_subscription(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
    Json(req): Json<CancelSubscriptionRequest>,
) -> Result<Json<Subscription>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    // Verify subscription exists and belongs to app
    let _existing = sqlx::query_as::<_, Subscription>(
        r#"
        SELECT
            s.id, s.app_id, s.billing_customer_id, s.tilled_subscription_id,
            s.plan_id, s.plan_name, s.price_cents, s.status, s.interval_unit, s.interval_count,
            s.billing_cycle_anchor, s.current_period_start, s.current_period_end,
            s.cancel_at_period_end, s.cancel_at, s.canceled_at, s.ended_at,
            s.payment_method_id, s.payment_method_type, s.metadata,
            s.update_source, s.updated_by, s.created_at, s.updated_at
        FROM billing_subscriptions s
        INNER JOIN billing_customers c ON s.billing_customer_id = c.id
        WHERE s.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching subscription: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch subscription: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Subscription {} not found", id),
            )),
        )
    })?;

    let cancel_at_period_end = req.cancel_at_period_end.unwrap_or(false);

    let subscription = if cancel_at_period_end {
        // Schedule cancellation at period end
        sqlx::query_as::<_, Subscription>(
            r#"
            UPDATE billing_subscriptions
            SET cancel_at_period_end = TRUE, updated_at = NOW()
            WHERE id = $1
            RETURNING
                id, app_id, billing_customer_id, tilled_subscription_id,
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    format!("Failed to cancel subscription: {}", e),
                )),
            )
        })?
    } else {
        // Immediate cancellation
        let now = chrono::Utc::now().naive_utc();
        sqlx::query_as::<_, Subscription>(
            r#"
            UPDATE billing_subscriptions
            SET status = 'canceled', canceled_at = $1, ended_at = $1, updated_at = NOW()
            WHERE id = $2
            RETURNING
                id, app_id, billing_customer_id, tilled_subscription_id,
                plan_id, plan_name, price_cents, status, interval_unit, interval_count,
                billing_cycle_anchor, current_period_start, current_period_end,
                cancel_at_period_end, cancel_at, canceled_at, ended_at,
                payment_method_id, payment_method_type, metadata,
                update_source, updated_by, created_at, updated_at
            "#,
        )
        .bind(&now)
        .bind(id)
        .fetch_one(&db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to cancel subscription: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    format!("Failed to cancel subscription: {}", e),
                )),
            )
        })?
    };

    tracing::info!(
        "Canceled subscription {} (at_period_end={})",
        id,
        cancel_at_period_end
    );

    // TODO: Sync with Tilled API

    Ok(Json(subscription))
}
