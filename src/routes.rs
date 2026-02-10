use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use sqlx::PgPool;

use crate::models::{
    CancelSubscriptionRequest, CaptureChargeRequest, Charge, CreateChargeRequest,
    CreateCustomerRequest, CreateInvoiceRequest, CreateSubscriptionRequest, Customer,
    ErrorResponse, FinalizeInvoiceRequest, Invoice, ListChargesQuery, ListCustomersQuery,
    ListInvoicesQuery, ListSubscriptionsQuery, Subscription, SubscriptionInterval,
    SubscriptionStatus, UpdateCustomerRequest, UpdateInvoiceRequest, UpdateSubscriptionRequest,
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
        // Invoice endpoints
        .route("/api/ar/invoices", post(create_invoice).get(list_invoices))
        .route(
            "/api/ar/invoices/{id}",
            get(get_invoice).put(update_invoice),
        )
        .route("/api/ar/invoices/{id}/finalize", post(finalize_invoice))
        // Charge endpoints
        .route("/api/ar/charges", post(create_charge).get(list_charges))
        .route("/api/ar/charges/{id}", get(get_charge))
        .route("/api/ar/charges/{id}/capture", post(capture_charge))
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

// ============================================================================
// INVOICE ENDPOINTS
// ============================================================================

/// POST /api/ar/invoices - Create a new invoice
async fn create_invoice(
    State(db): State<PgPool>,
    Json(req): Json<CreateInvoiceRequest>,
) -> Result<(StatusCode, Json<Invoice>), (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    // Validate required fields
    if req.amount_cents < 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "amount_cents must be non-negative",
            )),
        ));
    }

    let status = req.status.unwrap_or_else(|| "draft".to_string());
    let valid_statuses = ["draft", "open", "paid", "void", "uncollectible"];
    if !valid_statuses.contains(&status.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                format!("status must be one of: {}", valid_statuses.join(", ")),
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

    // Verify subscription exists if provided
    if let Some(subscription_id) = req.subscription_id {
        let _subscription = sqlx::query_as::<_, Subscription>(
            r#"
            SELECT
                s.id, s.app_id, s.billing_customer_id, s.tilled_subscription_id,
                s.plan_id, s.plan_name, s.price_cents, s.status, s.interval_unit, s.interval_count,
                s.billing_cycle_anchor, s.current_period_start, s.current_period_end,
                s.cancel_at_period_end, s.cancel_at, s.canceled_at, s.ended_at,
                s.payment_method_id, s.payment_method_type, s.metadata,
                s.update_source, s.updated_by, s.created_at, s.updated_at
            FROM billing_subscriptions s
            WHERE s.id = $1 AND s.app_id = $2 AND s.billing_customer_id = $3
            "#,
        )
        .bind(subscription_id)
        .bind(app_id)
        .bind(req.billing_customer_id)
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
                    format!(
                        "Subscription {} not found for customer {}",
                        subscription_id, req.billing_customer_id
                    ),
                )),
            )
        })?;
    }

    // Generate unique Tilled invoice ID
    let tilled_invoice_id = format!("in_{}_{}", app_id, uuid::Uuid::new_v4());
    let currency = req.currency.unwrap_or_else(|| "usd".to_string());

    // Create invoice
    let invoice = sqlx::query_as::<_, Invoice>(
        r#"
        INSERT INTO billing_invoices (
            app_id, tilled_invoice_id, billing_customer_id, subscription_id,
            status, amount_cents, currency, due_at, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, NOW(), NOW())
        RETURNING
            id, app_id, tilled_invoice_id, billing_customer_id, subscription_id,
            status, amount_cents, currency, due_at, paid_at, hosted_url, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(&tilled_invoice_id)
    .bind(req.billing_customer_id)
    .bind(req.subscription_id)
    .bind(&status)
    .bind(req.amount_cents)
    .bind(&currency)
    .bind(req.due_at)
    .bind(req.metadata)
    .bind(req.billing_period_start)
    .bind(req.billing_period_end)
    .bind(req.line_item_details)
    .bind(req.compliance_codes)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to create invoice: {}", e),
            )),
        )
    })?;

    tracing::info!(
        "Created invoice {} for customer {} (amount: {})",
        invoice.id,
        req.billing_customer_id,
        req.amount_cents
    );

    Ok((StatusCode::CREATED, Json(invoice)))
}

/// GET /api/ar/invoices/:id - Get invoice by ID
async fn get_invoice(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<Invoice>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    let invoice = sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.billing_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.created_at, i.updated_at
        FROM billing_invoices i
        INNER JOIN billing_customers c ON i.billing_customer_id = c.id
        WHERE i.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch invoice: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Invoice {} not found", id),
            )),
        )
    })?;

    Ok(Json(invoice))
}

/// GET /api/ar/invoices - List invoices (with optional filtering)
async fn list_invoices(
    State(db): State<PgPool>,
    Query(query): Query<ListInvoicesQuery>,
) -> Result<Json<Vec<Invoice>>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Build query based on filters
    let invoices = match (query.customer_id, query.subscription_id, query.status) {
        (Some(customer_id), _, Some(ref status)) => {
            sqlx::query_as::<_, Invoice>(
                r#"
                SELECT
                    i.id, i.app_id, i.tilled_invoice_id, i.billing_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.created_at, i.updated_at
                FROM billing_invoices i
                INNER JOIN billing_customers c ON i.billing_customer_id = c.id
                WHERE c.app_id = $1 AND i.billing_customer_id = $2 AND i.status = $3
                ORDER BY i.created_at DESC
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
        (Some(customer_id), _, None) => {
            sqlx::query_as::<_, Invoice>(
                r#"
                SELECT
                    i.id, i.app_id, i.tilled_invoice_id, i.billing_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.created_at, i.updated_at
                FROM billing_invoices i
                INNER JOIN billing_customers c ON i.billing_customer_id = c.id
                WHERE c.app_id = $1 AND i.billing_customer_id = $2
                ORDER BY i.created_at DESC
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
        (None, Some(subscription_id), _) => {
            sqlx::query_as::<_, Invoice>(
                r#"
                SELECT
                    i.id, i.app_id, i.tilled_invoice_id, i.billing_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.created_at, i.updated_at
                FROM billing_invoices i
                INNER JOIN billing_customers c ON i.billing_customer_id = c.id
                WHERE c.app_id = $1 AND i.subscription_id = $2
                ORDER BY i.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(app_id)
            .bind(subscription_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (None, None, Some(ref status)) => {
            sqlx::query_as::<_, Invoice>(
                r#"
                SELECT
                    i.id, i.app_id, i.tilled_invoice_id, i.billing_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.created_at, i.updated_at
                FROM billing_invoices i
                INNER JOIN billing_customers c ON i.billing_customer_id = c.id
                WHERE c.app_id = $1 AND i.status = $2
                ORDER BY i.created_at DESC
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
        (None, None, None) => {
            sqlx::query_as::<_, Invoice>(
                r#"
                SELECT
                    i.id, i.app_id, i.tilled_invoice_id, i.billing_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.created_at, i.updated_at
                FROM billing_invoices i
                INNER JOIN billing_customers c ON i.billing_customer_id = c.id
                WHERE c.app_id = $1
                ORDER BY i.created_at DESC
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
        tracing::error!("Database error listing invoices: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to list invoices: {}", e),
            )),
        )
    })?;

    Ok(Json(invoices))
}

/// PUT /api/ar/invoices/:id - Update invoice
async fn update_invoice(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
    Json(req): Json<UpdateInvoiceRequest>,
) -> Result<Json<Invoice>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    // Verify invoice exists and belongs to app
    let existing = sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.billing_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.created_at, i.updated_at
        FROM billing_invoices i
        INNER JOIN billing_customers c ON i.billing_customer_id = c.id
        WHERE i.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch invoice: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Invoice {} not found", id),
            )),
        )
    })?;

    // Validate at least one field is being updated
    if req.status.is_none()
        && req.amount_cents.is_none()
        && req.due_at.is_none()
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
    let status = req.status.unwrap_or(existing.status);
    let amount_cents = req.amount_cents.unwrap_or(existing.amount_cents);
    let due_at = req.due_at.or(existing.due_at);
    let metadata = req.metadata.or(existing.metadata);

    let invoice = sqlx::query_as::<_, Invoice>(
        r#"
        UPDATE billing_invoices
        SET status = $1, amount_cents = $2, due_at = $3, metadata = $4, updated_at = NOW()
        WHERE id = $5
        RETURNING
            id, app_id, tilled_invoice_id, billing_customer_id, subscription_id,
            status, amount_cents, currency, due_at, paid_at, hosted_url, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            created_at, updated_at
        "#,
    )
    .bind(&status)
    .bind(amount_cents)
    .bind(due_at)
    .bind(metadata)
    .bind(id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to update invoice: {}", e),
            )),
        )
    })?;

    tracing::info!("Updated invoice {}", id);

    Ok(Json(invoice))
}

/// POST /api/ar/invoices/:id/finalize - Mark invoice as finalized (open or paid)
async fn finalize_invoice(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
    Json(req): Json<FinalizeInvoiceRequest>,
) -> Result<Json<Invoice>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    // Verify invoice exists and belongs to app
    let existing = sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.billing_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.created_at, i.updated_at
        FROM billing_invoices i
        INNER JOIN billing_customers c ON i.billing_customer_id = c.id
        WHERE i.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch invoice: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Invoice {} not found", id),
            )),
        )
    })?;

    // Only draft invoices can be finalized to open
    if existing.status != "draft" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                format!("Cannot finalize invoice with status {}", existing.status),
            )),
        ));
    }

    let paid_at = req.paid_at.or_else(|| Some(chrono::Utc::now().naive_utc()));

    let invoice = sqlx::query_as::<_, Invoice>(
        r#"
        UPDATE billing_invoices
        SET status = 'open', paid_at = $1, updated_at = NOW()
        WHERE id = $2
        RETURNING
            id, app_id, tilled_invoice_id, billing_customer_id, subscription_id,
            status, amount_cents, currency, due_at, paid_at, hosted_url, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            created_at, updated_at
        "#,
    )
    .bind(paid_at)
    .bind(id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to finalize invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to finalize invoice: {}", e),
            )),
        )
    })?;

    tracing::info!("Finalized invoice {}", id);

    Ok(Json(invoice))
}

// ============================================================================
// CHARGE ENDPOINTS
// ============================================================================

/// POST /api/ar/charges - Create a new charge
async fn create_charge(
    State(db): State<PgPool>,
    Json(req): Json<CreateChargeRequest>,
) -> Result<(StatusCode, Json<Charge>), (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    // Validate required fields
    if req.amount_cents <= 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "amount_cents must be greater than 0",
            )),
        ));
    }

    if req.reason.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("validation_error", "reason is required")),
        ));
    }

    if req.reference_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "reference_id is required",
            )),
        ));
    }

    // Verify customer exists and belongs to app
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

    // Ensure default payment method exists
    if customer.default_payment_method_id.is_none() {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse::new(
                "conflict",
                "No default payment method on file",
            )),
        ));
    }

    // Check for duplicate reference_id
    let existing_charge = sqlx::query_as::<_, Charge>(
        r#"
        SELECT
            id, app_id, tilled_charge_id, invoice_id, billing_customer_id, subscription_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, failure_code, failure_message,
            product_type, quantity, service_frequency, weight_amount, location_reference,
            created_at, updated_at
        FROM billing_charges
        WHERE app_id = $1 AND reference_id = $2
        "#,
    )
    .bind(app_id)
    .bind(&req.reference_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error checking duplicate charge: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to check duplicate charge: {}", e),
            )),
        )
    })?;

    if let Some(charge) = existing_charge {
        tracing::info!(
            "Returning existing charge for duplicate reference_id: {}",
            req.reference_id
        );
        return Ok((StatusCode::OK, Json(charge)));
    }

    let currency = req.currency.unwrap_or_else(|| "usd".to_string());
    let charge_type = req.charge_type.unwrap_or_else(|| "one_time".to_string());

    // Create pending charge record
    let charge = sqlx::query_as::<_, Charge>(
        r#"
        INSERT INTO billing_charges (
            app_id, billing_customer_id, subscription_id, invoice_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, tilled_charge_id,
            created_at, updated_at
        )
        VALUES ($1, $2, NULL, NULL, 'pending', $3, $4, $5, $6, $7, $8, $9, $10, NULL, NOW(), NOW())
        RETURNING
            id, app_id, tilled_charge_id, invoice_id, billing_customer_id, subscription_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, failure_code, failure_message,
            product_type, quantity, service_frequency, weight_amount, location_reference,
            created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(req.billing_customer_id)
    .bind(req.amount_cents)
    .bind(&currency)
    .bind(&charge_type)
    .bind(&req.reason)
    .bind(&req.reference_id)
    .bind(req.service_date)
    .bind(req.note)
    .bind(req.metadata)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create charge: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to create charge: {}", e),
            )),
        )
    })?;

    // TODO: Integrate with Tilled API to create actual charge
    // For now, immediately mark as succeeded
    let charge = sqlx::query_as::<_, Charge>(
        r#"
        UPDATE billing_charges
        SET status = 'succeeded', updated_at = NOW()
        WHERE id = $1
        RETURNING
            id, app_id, tilled_charge_id, invoice_id, billing_customer_id, subscription_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, failure_code, failure_message,
            product_type, quantity, service_frequency, weight_amount, location_reference,
            created_at, updated_at
        "#,
    )
    .bind(charge.id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update charge status: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to update charge: {}", e),
            )),
        )
    })?;

    tracing::info!(
        "Created charge {} for customer {} (amount: {})",
        charge.id,
        req.billing_customer_id,
        req.amount_cents
    );

    Ok((StatusCode::CREATED, Json(charge)))
}

/// GET /api/ar/charges/:id - Get charge by ID
async fn get_charge(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<Charge>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    let charge = sqlx::query_as::<_, Charge>(
        r#"
        SELECT
            ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.billing_customer_id, ch.subscription_id,
            ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
            ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
            ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
            ch.created_at, ch.updated_at
        FROM billing_charges ch
        INNER JOIN billing_customers c ON ch.billing_customer_id = c.id
        WHERE ch.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching charge: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch charge: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Charge {} not found", id),
            )),
        )
    })?;

    Ok(Json(charge))
}

/// GET /api/ar/charges - List charges (with optional filtering)
async fn list_charges(
    State(db): State<PgPool>,
    Query(query): Query<ListChargesQuery>,
) -> Result<Json<Vec<Charge>>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Build query based on filters
    let charges = match (query.customer_id, query.invoice_id, query.status) {
        (Some(customer_id), _, Some(ref status)) => {
            sqlx::query_as::<_, Charge>(
                r#"
                SELECT
                    ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.billing_customer_id, ch.subscription_id,
                    ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
                    ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
                    ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
                    ch.created_at, ch.updated_at
                FROM billing_charges ch
                INNER JOIN billing_customers c ON ch.billing_customer_id = c.id
                WHERE c.app_id = $1 AND ch.billing_customer_id = $2 AND ch.status = $3
                ORDER BY ch.created_at DESC
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
        (Some(customer_id), _, None) => {
            sqlx::query_as::<_, Charge>(
                r#"
                SELECT
                    ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.billing_customer_id, ch.subscription_id,
                    ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
                    ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
                    ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
                    ch.created_at, ch.updated_at
                FROM billing_charges ch
                INNER JOIN billing_customers c ON ch.billing_customer_id = c.id
                WHERE c.app_id = $1 AND ch.billing_customer_id = $2
                ORDER BY ch.created_at DESC
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
        (None, Some(invoice_id), _) => {
            sqlx::query_as::<_, Charge>(
                r#"
                SELECT
                    ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.billing_customer_id, ch.subscription_id,
                    ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
                    ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
                    ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
                    ch.created_at, ch.updated_at
                FROM billing_charges ch
                INNER JOIN billing_customers c ON ch.billing_customer_id = c.id
                WHERE c.app_id = $1 AND ch.invoice_id = $2
                ORDER BY ch.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(app_id)
            .bind(invoice_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (None, None, Some(ref status)) => {
            sqlx::query_as::<_, Charge>(
                r#"
                SELECT
                    ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.billing_customer_id, ch.subscription_id,
                    ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
                    ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
                    ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
                    ch.created_at, ch.updated_at
                FROM billing_charges ch
                INNER JOIN billing_customers c ON ch.billing_customer_id = c.id
                WHERE c.app_id = $1 AND ch.status = $2
                ORDER BY ch.created_at DESC
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
        (None, None, None) => {
            sqlx::query_as::<_, Charge>(
                r#"
                SELECT
                    ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.billing_customer_id, ch.subscription_id,
                    ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
                    ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
                    ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
                    ch.created_at, ch.updated_at
                FROM billing_charges ch
                INNER JOIN billing_customers c ON ch.billing_customer_id = c.id
                WHERE c.app_id = $1
                ORDER BY ch.created_at DESC
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
        tracing::error!("Database error listing charges: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to list charges: {}", e),
            )),
        )
    })?;

    Ok(Json(charges))
}

/// POST /api/ar/charges/:id/capture - Capture an authorized charge
async fn capture_charge(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
    Json(req): Json<CaptureChargeRequest>,
) -> Result<Json<Charge>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "default_app"; // Placeholder

    // Verify charge exists and belongs to app
    let existing = sqlx::query_as::<_, Charge>(
        r#"
        SELECT
            ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.billing_customer_id, ch.subscription_id,
            ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
            ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
            ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
            ch.created_at, ch.updated_at
        FROM billing_charges ch
        INNER JOIN billing_customers c ON ch.billing_customer_id = c.id
        WHERE ch.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching charge: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch charge: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("not_found", format!("Charge {} not found", id))),
        )
    })?;

    // Only authorized charges can be captured
    if existing.status != "authorized" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                format!("Cannot capture charge with status {}", existing.status),
            )),
        ));
    }

    // Use provided amount or existing amount
    let capture_amount = req.amount_cents.unwrap_or(existing.amount_cents);

    // TODO: Integrate with Tilled API to capture the charge
    // For now, update status to captured

    let charge = sqlx::query_as::<_, Charge>(
        r#"
        UPDATE billing_charges
        SET status = 'captured', amount_cents = $1, updated_at = NOW()
        WHERE id = $2
        RETURNING
            id, app_id, tilled_charge_id, invoice_id, billing_customer_id, subscription_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, failure_code, failure_message,
            product_type, quantity, service_frequency, weight_amount, location_reference,
            created_at, updated_at
        "#,
    )
    .bind(capture_amount)
    .bind(id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to capture charge: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to capture charge: {}", e),
            )),
        )
    })?;

    tracing::info!("Captured charge {} (amount: {})", id, capture_amount);

    Ok(Json(charge))
}
