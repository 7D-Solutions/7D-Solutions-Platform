use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    routing::{get, post},
    Json, Router,
};
use chrono::Datelike;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::PgPool;

use crate::idempotency::{check_idempotency, log_event_async};
use crate::models::{
    AddPaymentMethodRequest, CancelSubscriptionRequest, CaptureChargeRequest, Charge,
    CreateChargeRequest, CreateCustomerRequest, CreateInvoiceRequest, CreateRefundRequest,
    CreateSubscriptionRequest, Customer, Dispute, ErrorResponse, Event, FinalizeInvoiceRequest,
    Invoice, ListChargesQuery, ListCustomersQuery, ListDisputesQuery, ListEventsQuery,
    ListInvoicesQuery, ListPaymentMethodsQuery, ListRefundsQuery, ListSubscriptionsQuery,
    ListWebhooksQuery, PaymentMethod, Refund, ReplayWebhookRequest, SubmitDisputeEvidenceRequest,
    Subscription, SubscriptionInterval, SubscriptionStatus, TilledWebhookEvent,
    UpdateCustomerRequest, UpdateInvoiceRequest, UpdatePaymentMethodRequest,
    UpdateSubscriptionRequest, Webhook, WebhookStatus,
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
        // Refund endpoints
        .route("/api/ar/refunds", post(create_refund).get(list_refunds))
        .route("/api/ar/refunds/{id}", get(get_refund))
        // Dispute endpoints
        .route("/api/ar/disputes", get(list_disputes))
        .route("/api/ar/disputes/{id}", get(get_dispute))
        .route("/api/ar/disputes/{id}/evidence", post(submit_dispute_evidence))
        // Payment method endpoints
        .route(
            "/api/ar/payment-methods",
            post(add_payment_method).get(list_payment_methods),
        )
        .route(
            "/api/ar/payment-methods/{id}",
            get(get_payment_method)
                .put(update_payment_method)
                .delete(delete_payment_method),
        )
        .route(
            "/api/ar/payment-methods/{id}/set-default",
            post(set_default_payment_method),
        )
        // Webhook endpoints
        .route("/api/ar/webhooks/tilled", post(receive_tilled_webhook))
        .route("/api/ar/webhooks", get(list_webhooks))
        .route("/api/ar/webhooks/{id}", get(get_webhook))
        .route("/api/ar/webhooks/{id}/replay", post(replay_webhook))
        // Event log endpoints
        .route("/api/ar/events", get(list_events))
        .route("/api/ar/events/{id}", get(get_event))
        .with_state(db.clone())
        .layer(middleware::from_fn_with_state(db, check_idempotency))
}

/// POST /api/ar/customers - Create a new customer
async fn create_customer(
    State(db): State<PgPool>,
    Json(req): Json<CreateCustomerRequest>,
) -> Result<(StatusCode, Json<Customer>), (StatusCode, Json<ErrorResponse>)> {
    // Validate email
    let email = match &req.email {
        Some(e) if !e.trim().is_empty() => e.trim(),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new("validation_error", "Email is required")),
            ));
        }
    };

    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    // Check for duplicate email
    let existing: Option<(i32,)> = sqlx::query_as(
        "SELECT id FROM ar_customers WHERE app_id = $1 AND email = $2 LIMIT 1",
    )
    .bind(app_id)
    .bind(email)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to check duplicate email: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to validate email",
            )),
        )
    })?;

    if existing.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse::new(
                "duplicate_email",
                "A customer with this email already exists",
            )),
        ));
    }

    // Create customer in database (local-first pattern)
    let customer = sqlx::query_as::<_, Customer>(
        r#"
        INSERT INTO ar_customers (
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
    .bind(email)
    .bind(req.name)
    .bind(req.metadata)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create customer: {:?}", e);

        // Check for unique constraint violation (duplicate email)
        if let sqlx::Error::Database(db_err) = &e {
            if db_err.code() == Some(std::borrow::Cow::Borrowed("23505")) {
                return (
                    StatusCode::CONFLICT,
                    Json(ErrorResponse::new(
                        "duplicate_email",
                        "A customer with this email already exists",
                    )),
                );
            }
        }

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
        UPDATE ar_customers
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

    // Log event asynchronously
    log_event_async(
        db.clone(),
        app_id.to_string(),
        "customer.created".to_string(),
        "api".to_string(),
        Some("customer".to_string()),
        Some(customer.id.to_string()),
        Some(serde_json::to_value(&customer).unwrap_or_default()),
    );

    Ok((StatusCode::CREATED, Json(customer)))
}

/// GET /api/ar/customers/:id - Get customer by ID
async fn get_customer(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<Customer>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    let customer = sqlx::query_as::<_, Customer>(
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
    let app_id = "test-app"; // Placeholder

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
            FROM ar_customers
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
    let app_id = "test-app"; // Placeholder

    // Verify customer exists and belongs to app
    let existing = sqlx::query_as::<_, Customer>(
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
        UPDATE ar_customers
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
    let app_id = "test-app"; // Placeholder

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
        FROM ar_customers
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(req.ar_customer_id)
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
                format!("Customer {} not found", req.ar_customer_id),
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
        INSERT INTO ar_subscriptions (
            app_id, ar_customer_id, tilled_subscription_id,
            plan_id, plan_name, price_cents, status, interval_unit, interval_count,
            current_period_start, current_period_end, cancel_at_period_end,
            payment_method_id, payment_method_type, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, NOW(), NOW())
        RETURNING
            id, app_id, ar_customer_id, tilled_subscription_id,
            plan_id, plan_name, price_cents, status, interval_unit, interval_count,
            billing_cycle_anchor, current_period_start, current_period_end,
            cancel_at_period_end, cancel_at, canceled_at, ended_at,
            payment_method_id, payment_method_type, metadata,
            update_source, updated_by, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(req.ar_customer_id)
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
        req.ar_customer_id
    );

    Ok((StatusCode::CREATED, Json(subscription)))
}

/// GET /api/ar/subscriptions/:id - Get subscription by ID
async fn get_subscription(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<Subscription>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

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
    let app_id = "test-app"; // Placeholder

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
    let app_id = "test-app"; // Placeholder

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
        UPDATE ar_subscriptions
        SET plan_id = $1, plan_name = $2, price_cents = $3, metadata = $4, updated_at = NOW()
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
    let app_id = "test-app"; // Placeholder

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
            UPDATE ar_subscriptions
            SET status = 'canceled', canceled_at = $1, ended_at = $1, updated_at = NOW()
            WHERE id = $2
            RETURNING
                id, app_id, ar_customer_id, tilled_subscription_id,
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
    let app_id = "test-app"; // Placeholder

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
        FROM ar_customers
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(req.ar_customer_id)
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
                format!("Customer {} not found", req.ar_customer_id),
            )),
        )
    })?;

    // Verify subscription exists if provided
    if let Some(subscription_id) = req.subscription_id {
        let _subscription = sqlx::query_as::<_, Subscription>(
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
        .bind(req.ar_customer_id)
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
                        subscription_id, req.ar_customer_id
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
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, due_at, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, NOW(), NOW())
        RETURNING
            id, app_id, tilled_invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, due_at, paid_at, hosted_url, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(&tilled_invoice_id)
    .bind(req.ar_customer_id)
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
        req.ar_customer_id,
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
    let app_id = "test-app"; // Placeholder

    let invoice = sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.created_at, i.updated_at
        FROM ar_invoices i
        INNER JOIN ar_customers c ON i.ar_customer_id = c.id
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
    let app_id = "test-app"; // Placeholder

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Build query based on filters
    let invoices = match (query.customer_id, query.subscription_id, query.status) {
        (Some(customer_id), _, Some(ref status)) => {
            sqlx::query_as::<_, Invoice>(
                r#"
                SELECT
                    i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.created_at, i.updated_at
                FROM ar_invoices i
                INNER JOIN ar_customers c ON i.ar_customer_id = c.id
                WHERE c.app_id = $1 AND i.ar_customer_id = $2 AND i.status = $3
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
                    i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.created_at, i.updated_at
                FROM ar_invoices i
                INNER JOIN ar_customers c ON i.ar_customer_id = c.id
                WHERE c.app_id = $1 AND i.ar_customer_id = $2
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
                    i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.created_at, i.updated_at
                FROM ar_invoices i
                INNER JOIN ar_customers c ON i.ar_customer_id = c.id
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
                    i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.created_at, i.updated_at
                FROM ar_invoices i
                INNER JOIN ar_customers c ON i.ar_customer_id = c.id
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
                    i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.created_at, i.updated_at
                FROM ar_invoices i
                INNER JOIN ar_customers c ON i.ar_customer_id = c.id
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
    let app_id = "test-app"; // Placeholder

    // Verify invoice exists and belongs to app
    let existing = sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.created_at, i.updated_at
        FROM ar_invoices i
        INNER JOIN ar_customers c ON i.ar_customer_id = c.id
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
        UPDATE ar_invoices
        SET status = $1, amount_cents = $2, due_at = $3, metadata = $4, updated_at = NOW()
        WHERE id = $5
        RETURNING
            id, app_id, tilled_invoice_id, ar_customer_id, subscription_id,
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
    let app_id = "test-app"; // Placeholder

    // Verify invoice exists and belongs to app
    let existing = sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.created_at, i.updated_at
        FROM ar_invoices i
        INNER JOIN ar_customers c ON i.ar_customer_id = c.id
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
        UPDATE ar_invoices
        SET status = 'open', paid_at = $1, updated_at = NOW()
        WHERE id = $2
        RETURNING
            id, app_id, tilled_invoice_id, ar_customer_id, subscription_id,
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
    let app_id = "test-app"; // Placeholder

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
        FROM ar_customers
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(req.ar_customer_id)
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
                format!("Customer {} not found", req.ar_customer_id),
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
            id, app_id, tilled_charge_id, invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, failure_code, failure_message,
            product_type, quantity, service_frequency, weight_amount, location_reference,
            created_at, updated_at
        FROM ar_charges
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
        INSERT INTO ar_charges (
            app_id, ar_customer_id, subscription_id, invoice_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, tilled_charge_id,
            created_at, updated_at
        )
        VALUES ($1, $2, NULL, NULL, 'pending', $3, $4, $5, $6, $7, $8, $9, $10, NULL, NOW(), NOW())
        RETURNING
            id, app_id, tilled_charge_id, invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, failure_code, failure_message,
            product_type, quantity, service_frequency, weight_amount, location_reference,
            created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(req.ar_customer_id)
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
    // For now, mark as authorized and set mock tilled_charge_id for tests
    let mock_tilled_id = format!("pi_test_{}", charge.id);
    let charge = sqlx::query_as::<_, Charge>(
        r#"
        UPDATE ar_charges
        SET status = 'authorized', tilled_charge_id = $2, updated_at = NOW()
        WHERE id = $1
        RETURNING
            id, app_id, tilled_charge_id, invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, charge_type, reason, reference_id,
            service_date, note, metadata, failure_code, failure_message,
            product_type, quantity, service_frequency, weight_amount, location_reference,
            created_at, updated_at
        "#,
    )
    .bind(charge.id)
    .bind(&mock_tilled_id)
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
        req.ar_customer_id,
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
    let app_id = "test-app"; // Placeholder

    let charge = sqlx::query_as::<_, Charge>(
        r#"
        SELECT
            ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.ar_customer_id, ch.subscription_id,
            ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
            ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
            ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
            ch.created_at, ch.updated_at
        FROM ar_charges ch
        INNER JOIN ar_customers c ON ch.ar_customer_id = c.id
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
    let app_id = "test-app"; // Placeholder

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Build query based on filters
    let charges = match (query.customer_id, query.invoice_id, query.status) {
        (Some(customer_id), _, Some(ref status)) => {
            sqlx::query_as::<_, Charge>(
                r#"
                SELECT
                    ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.ar_customer_id, ch.subscription_id,
                    ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
                    ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
                    ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
                    ch.created_at, ch.updated_at
                FROM ar_charges ch
                INNER JOIN ar_customers c ON ch.ar_customer_id = c.id
                WHERE c.app_id = $1 AND ch.ar_customer_id = $2 AND ch.status = $3
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
                    ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.ar_customer_id, ch.subscription_id,
                    ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
                    ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
                    ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
                    ch.created_at, ch.updated_at
                FROM ar_charges ch
                INNER JOIN ar_customers c ON ch.ar_customer_id = c.id
                WHERE c.app_id = $1 AND ch.ar_customer_id = $2
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
                    ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.ar_customer_id, ch.subscription_id,
                    ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
                    ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
                    ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
                    ch.created_at, ch.updated_at
                FROM ar_charges ch
                INNER JOIN ar_customers c ON ch.ar_customer_id = c.id
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
                    ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.ar_customer_id, ch.subscription_id,
                    ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
                    ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
                    ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
                    ch.created_at, ch.updated_at
                FROM ar_charges ch
                INNER JOIN ar_customers c ON ch.ar_customer_id = c.id
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
                    ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.ar_customer_id, ch.subscription_id,
                    ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
                    ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
                    ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
                    ch.created_at, ch.updated_at
                FROM ar_charges ch
                INNER JOIN ar_customers c ON ch.ar_customer_id = c.id
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
    let app_id = "test-app"; // Placeholder

    // Verify charge exists and belongs to app
    let existing = sqlx::query_as::<_, Charge>(
        r#"
        SELECT
            ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.ar_customer_id, ch.subscription_id,
            ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
            ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
            ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
            ch.created_at, ch.updated_at
        FROM ar_charges ch
        INNER JOIN ar_customers c ON ch.ar_customer_id = c.id
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
    // For now, update status to succeeded (captured and processing)

    let charge = sqlx::query_as::<_, Charge>(
        r#"
        UPDATE ar_charges
        SET status = 'succeeded', amount_cents = $1, updated_at = NOW()
        WHERE id = $2
        RETURNING
            id, app_id, tilled_charge_id, invoice_id, ar_customer_id, subscription_id,
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

// ============================================================================
// REFUND ENDPOINTS
// ============================================================================

/// POST /api/ar/refunds - Create a refund for a charge
async fn create_refund(
    State(db): State<PgPool>,
    Json(req): Json<CreateRefundRequest>,
) -> Result<(StatusCode, Json<Refund>), (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

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

    if req.reference_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "reference_id is required",
            )),
        ));
    }

    // Check for duplicate reference_id (domain-level idempotency)
    let existing_refund = sqlx::query_as::<_, Refund>(
        r#"
        SELECT
            id, app_id, ar_customer_id, charge_id, tilled_refund_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            failure_code, failure_message, created_at, updated_at
        FROM ar_refunds
        WHERE app_id = $1 AND reference_id = $2
        "#,
    )
    .bind(app_id)
    .bind(&req.reference_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error checking duplicate refund: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to check for duplicate refund",
            )),
        )
    })?;

    if let Some(refund) = existing_refund {
        tracing::info!(
            "Returning existing refund for duplicate reference_id: {}",
            req.reference_id
        );
        return Ok((StatusCode::OK, Json(refund)));
    }

    // Load charge with app_id scoping
    let charge = sqlx::query_as::<_, Charge>(
        r#"
        SELECT
            ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.ar_customer_id, ch.subscription_id,
            ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
            ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
            ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
            ch.created_at, ch.updated_at
        FROM ar_charges ch
        INNER JOIN ar_customers c ON ch.ar_customer_id = c.id
        WHERE ch.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(req.charge_id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching charge: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch charge",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("not_found", "Charge not found")),
        )
    })?;

    // Ensure charge has been settled in processor
    if charge.tilled_charge_id.is_none() {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse::new(
                "conflict",
                "Charge not settled in processor",
            )),
        ));
    }

    // Validate refund amount does not exceed charge amount
    if req.amount_cents > charge.amount_cents {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                format!(
                    "Refund amount ({}) exceeds charge amount ({})",
                    req.amount_cents, charge.amount_cents
                ),
            )),
        ));
    }

    // Calculate total already refunded
    let total_refunded: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(amount_cents), 0)
        FROM ar_refunds
        WHERE charge_id = $1 AND app_id = $2 AND status IN ('pending', 'succeeded')
        "#,
    )
    .bind(req.charge_id)
    .bind(app_id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error calculating refunded amount: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to calculate refunded amount",
            )),
        )
    })?;

    let total_refunded = total_refunded.unwrap_or(0) as i32;
    let remaining_refundable = charge.amount_cents - total_refunded;

    if req.amount_cents > remaining_refundable {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                format!(
                    "Refund amount ({}) exceeds remaining refundable amount ({}). Total already refunded: {}",
                    req.amount_cents, remaining_refundable, total_refunded
                ),
            )),
        ));
    }

    // Create pending refund record
    let refund = sqlx::query_as::<_, Refund>(
        r#"
        INSERT INTO ar_refunds (
            app_id, ar_customer_id, charge_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, 'pending', $5, $6, $7, $8, $9, $10, NOW(), NOW())
        RETURNING
            id, app_id, ar_customer_id, charge_id, tilled_refund_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            failure_code, failure_message, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(charge.ar_customer_id)
    .bind(req.charge_id)
    .bind(&charge.tilled_charge_id)
    .bind(req.amount_cents)
    .bind(req.currency.as_deref().unwrap_or("usd"))
    .bind(&req.reason)
    .bind(&req.reference_id)
    .bind(&req.note)
    .bind(&req.metadata)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create refund: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to create refund: {}", e),
            )),
        )
    })?;

    // TODO: Integrate with Tilled API to create actual refund
    // For now, we'll update status to succeeded
    let refund = sqlx::query_as::<_, Refund>(
        r#"
        UPDATE ar_refunds
        SET status = 'succeeded', tilled_refund_id = $1, updated_at = NOW()
        WHERE id = $2
        RETURNING
            id, app_id, ar_customer_id, charge_id, tilled_refund_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            failure_code, failure_message, created_at, updated_at
        "#,
    )
    .bind(format!("ref_mock_{}", refund.id))  // Mock Tilled refund ID
    .bind(refund.id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update refund: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to update refund",
            )),
        )
    })?;

    tracing::info!(
        "Created refund {} for charge {} (amount: {})",
        refund.id,
        req.charge_id,
        req.amount_cents
    );

    Ok((StatusCode::CREATED, Json(refund)))
}

/// GET /api/ar/refunds/{id} - Get a specific refund
async fn get_refund(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<Refund>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    let refund = sqlx::query_as::<_, Refund>(
        r#"
        SELECT
            id, app_id, ar_customer_id, charge_id, tilled_refund_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            failure_code, failure_message, created_at, updated_at
        FROM ar_refunds
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching refund: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch refund",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("not_found", format!("Refund {} not found", id))),
        )
    })?;

    Ok(Json(refund))
}

/// GET /api/ar/refunds - List refunds with optional filters
async fn list_refunds(
    State(db): State<PgPool>,
    Query(query): Query<ListRefundsQuery>,
) -> Result<Json<Vec<Refund>>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);

    // Build dynamic query based on filters
    let mut sql = String::from(
        r#"
        SELECT
            id, app_id, ar_customer_id, charge_id, tilled_refund_id, tilled_charge_id,
            status, amount_cents, currency, reason, reference_id, note, metadata,
            failure_code, failure_message, created_at, updated_at
        FROM ar_refunds
        WHERE app_id = $1
        "#,
    );

    let mut bind_index = 2;
    if query.charge_id.is_some() {
        sql.push_str(&format!(" AND charge_id = ${}", bind_index));
        bind_index += 1;
    }
    if query.customer_id.is_some() {
        sql.push_str(&format!(" AND ar_customer_id = ${}", bind_index));
        bind_index += 1;
    }
    if query.status.is_some() {
        sql.push_str(&format!(" AND status = ${}", bind_index));
        bind_index += 1;
    }

    sql.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${} OFFSET ${}",
        bind_index,
        bind_index + 1
    ));

    let mut query_builder = sqlx::query_as::<_, Refund>(&sql).bind(app_id);

    if let Some(charge_id) = query.charge_id {
        query_builder = query_builder.bind(charge_id);
    }
    if let Some(customer_id) = query.customer_id {
        query_builder = query_builder.bind(customer_id);
    }
    if let Some(status) = query.status {
        query_builder = query_builder.bind(status);
    }

    let refunds = query_builder
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
        .await
        .map_err(|e| {
            tracing::error!("Database error listing refunds: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    "Failed to list refunds",
                )),
            )
        })?;

    Ok(Json(refunds))
}

// ============================================================================
// DISPUTE ENDPOINTS
// ============================================================================

/// GET /api/ar/disputes - List disputes with optional filters
async fn list_disputes(
    State(db): State<PgPool>,
    Query(query): Query<ListDisputesQuery>,
) -> Result<Json<Vec<Dispute>>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);

    // Build dynamic query based on filters
    let mut sql = String::from(
        r#"
        SELECT
            id, app_id, tilled_dispute_id, tilled_charge_id, charge_id,
            status, amount_cents, currency, reason, reason_code,
            evidence_due_by, opened_at, closed_at, created_at, updated_at
        FROM ar_disputes
        WHERE app_id = $1
        "#,
    );

    let mut bind_index = 2;
    if query.charge_id.is_some() {
        sql.push_str(&format!(" AND charge_id = ${}", bind_index));
        bind_index += 1;
    }
    if query.status.is_some() {
        sql.push_str(&format!(" AND status = ${}", bind_index));
        bind_index += 1;
    }

    sql.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${} OFFSET ${}",
        bind_index,
        bind_index + 1
    ));

    let mut query_builder = sqlx::query_as::<_, Dispute>(&sql).bind(app_id);

    if let Some(charge_id) = query.charge_id {
        query_builder = query_builder.bind(charge_id);
    }
    if let Some(status) = query.status {
        query_builder = query_builder.bind(status);
    }

    let disputes = query_builder
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
        .await
        .map_err(|e| {
            tracing::error!("Database error listing disputes: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    "Failed to list disputes",
                )),
            )
        })?;

    Ok(Json(disputes))
}

/// GET /api/ar/disputes/{id} - Get a specific dispute
async fn get_dispute(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<Dispute>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    let dispute = sqlx::query_as::<_, Dispute>(
        r#"
        SELECT
            id, app_id, tilled_dispute_id, tilled_charge_id, charge_id,
            status, amount_cents, currency, reason, reason_code,
            evidence_due_by, opened_at, closed_at, created_at, updated_at
        FROM ar_disputes
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching dispute: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch dispute",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("not_found", format!("Dispute {} not found", id))),
        )
    })?;

    Ok(Json(dispute))
}

/// POST /api/ar/disputes/{id}/evidence - Submit evidence for a dispute
async fn submit_dispute_evidence(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
    Json(_req): Json<SubmitDisputeEvidenceRequest>,
) -> Result<Json<Dispute>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    // Verify dispute exists and belongs to app
    let dispute = sqlx::query_as::<_, Dispute>(
        r#"
        SELECT
            id, app_id, tilled_dispute_id, tilled_charge_id, charge_id,
            status, amount_cents, currency, reason, reason_code,
            evidence_due_by, opened_at, closed_at, created_at, updated_at
        FROM ar_disputes
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching dispute: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch dispute",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("not_found", format!("Dispute {} not found", id))),
        )
    })?;

    // Check if evidence is still acceptable (before due date)
    if let Some(evidence_due_by) = dispute.evidence_due_by {
        if chrono::Utc::now().naive_utc() > evidence_due_by {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new(
                    "validation_error",
                    "Evidence submission deadline has passed",
                )),
            ));
        }
    }

    // TODO: Integrate with Tilled API to submit evidence
    // For now, just log it
    tracing::info!(
        "Submitted evidence for dispute {} (Tilled ID: {})",
        id,
        dispute.tilled_dispute_id
    );

    // Return the dispute unchanged (in real implementation, it would be updated)
    Ok(Json(dispute))
}

// ============================================================================
// PAYMENT METHOD ENDPOINTS
// ============================================================================

/// POST /api/ar/payment-methods - Add a new payment method
async fn add_payment_method(
    State(db): State<PgPool>,
    Json(req): Json<AddPaymentMethodRequest>,
) -> Result<(StatusCode, Json<PaymentMethod>), (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    // Validate required fields
    if req.tilled_payment_method_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "tilled_payment_method_id is required",
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
        FROM ar_customers
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(req.ar_customer_id)
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
                format!("Customer {} not found", req.ar_customer_id),
            )),
        )
    })?;

    // Check if payment method already exists (upsert pattern)
    let existing = sqlx::query_as::<_, PaymentMethod>(
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
    .bind(&req.tilled_payment_method_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error checking payment method: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to check payment method: {}", e),
            )),
        )
    })?;

    let payment_method = if let Some(_pm) = existing {
        // Update existing record (reactivate if deleted)
        sqlx::query_as::<_, PaymentMethod>(
            r#"
            UPDATE ar_payment_methods
            SET app_id = $1, ar_customer_id = $2, status = 'pending',
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
        .bind(req.ar_customer_id)
        .bind(&req.tilled_payment_method_id)
        .fetch_one(&db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update payment method: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    format!("Failed to update payment method: {}", e),
                )),
            )
        })?
    } else {
        // Create new payment method record (local-first pattern)
        sqlx::query_as::<_, PaymentMethod>(
            r#"
            INSERT INTO ar_payment_methods (
                app_id, ar_customer_id, tilled_payment_method_id,
                type, status, is_default, metadata, created_at, updated_at
            )
            VALUES ($1, $2, $3, 'card', 'pending', FALSE, '{}', NOW(), NOW())
            RETURNING
                id, app_id, ar_customer_id, tilled_payment_method_id,
                status, type, brand, last4, exp_month, exp_year,
                bank_name, bank_last4, is_default, metadata,
                deleted_at, created_at, updated_at
            "#,
        )
        .bind(app_id)
        .bind(req.ar_customer_id)
        .bind(&req.tilled_payment_method_id)
        .fetch_one(&db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create payment method: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    format!("Failed to create payment method: {}", e),
                )),
            )
        })?
    };

    // TODO: Integrate with Tilled API to:
    // 1. Attach payment method to customer
    // 2. Fetch full payment method details (brand, last4, etc.)
    // For now, mark as active immediately
    let payment_method = sqlx::query_as::<_, PaymentMethod>(
        r#"
        UPDATE ar_payment_methods
        SET status = 'active', updated_at = NOW()
        WHERE id = $1
        RETURNING
            id, app_id, ar_customer_id, tilled_payment_method_id,
            status, type, brand, last4, exp_month, exp_year,
            bank_name, bank_last4, is_default, metadata,
            deleted_at, created_at, updated_at
        "#,
    )
    .bind(payment_method.id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update payment method status: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to update payment method: {}", e),
            )),
        )
    })?;

    tracing::info!(
        "Added payment method {} for customer {}",
        payment_method.id,
        req.ar_customer_id
    );

    Ok((StatusCode::CREATED, Json(payment_method)))
}

/// GET /api/ar/payment-methods/:id - Get payment method by ID
async fn get_payment_method(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<PaymentMethod>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    let payment_method = sqlx::query_as::<_, PaymentMethod>(
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
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching payment method: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch payment method: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Payment method {} not found", id),
            )),
        )
    })?;

    Ok(Json(payment_method))
}

/// GET /api/ar/payment-methods - List payment methods (with optional filtering)
async fn list_payment_methods(
    State(db): State<PgPool>,
    Query(query): Query<ListPaymentMethodsQuery>,
) -> Result<Json<Vec<PaymentMethod>>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Build query based on filters
    let payment_methods = match (query.customer_id, query.status) {
        (Some(customer_id), Some(ref status)) => {
            // Filter by both customer and status
            sqlx::query_as::<_, PaymentMethod>(
                r#"
                SELECT
                    pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
                    pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
                    pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
                    pm.deleted_at, pm.created_at, pm.updated_at
                FROM ar_payment_methods pm
                INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
                WHERE c.app_id = $1 AND pm.ar_customer_id = $2 AND pm.status = $3
                    AND pm.deleted_at IS NULL
                ORDER BY pm.is_default DESC, pm.created_at DESC
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
            // Filter by customer only (most common case)
            sqlx::query_as::<_, PaymentMethod>(
                r#"
                SELECT
                    pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
                    pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
                    pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
                    pm.deleted_at, pm.created_at, pm.updated_at
                FROM ar_payment_methods pm
                INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
                WHERE c.app_id = $1 AND pm.ar_customer_id = $2 AND pm.deleted_at IS NULL
                ORDER BY pm.is_default DESC, pm.created_at DESC
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
        (None, Some(ref status)) => {
            // Filter by status only (unusual case)
            sqlx::query_as::<_, PaymentMethod>(
                r#"
                SELECT
                    pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
                    pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
                    pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
                    pm.deleted_at, pm.created_at, pm.updated_at
                FROM ar_payment_methods pm
                INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
                WHERE c.app_id = $1 AND pm.status = $2 AND pm.deleted_at IS NULL
                ORDER BY pm.is_default DESC, pm.created_at DESC
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
            // List all for app (rare case)
            sqlx::query_as::<_, PaymentMethod>(
                r#"
                SELECT
                    pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
                    pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
                    pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
                    pm.deleted_at, pm.created_at, pm.updated_at
                FROM ar_payment_methods pm
                INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
                WHERE c.app_id = $1 AND pm.deleted_at IS NULL
                ORDER BY pm.is_default DESC, pm.created_at DESC
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
        tracing::error!("Database error listing payment methods: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to list payment methods: {}", e),
            )),
        )
    })?;

    Ok(Json(payment_methods))
}

/// PUT /api/ar/payment-methods/:id - Update payment method
async fn update_payment_method(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
    Json(req): Json<UpdatePaymentMethodRequest>,
) -> Result<Json<PaymentMethod>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    // Verify payment method exists and belongs to app
    let existing = sqlx::query_as::<_, PaymentMethod>(
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
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching payment method: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch payment method: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Payment method {} not found", id),
            )),
        )
    })?;

    // Validate at least one field is being updated
    if req.metadata.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "No valid fields to update",
            )),
        ));
    }

    // Update metadata only (other fields come from Tilled)
    let metadata = req.metadata.or(existing.metadata);

    let payment_method = sqlx::query_as::<_, PaymentMethod>(
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
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update payment method: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to update payment method: {}", e),
            )),
        )
    })?;

    tracing::info!("Updated payment method {}", id);

    Ok(Json(payment_method))
}

/// DELETE /api/ar/payment-methods/:id - Delete payment method (soft delete)
async fn delete_payment_method(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    // Verify payment method exists and belongs to app
    let payment_method = sqlx::query_as::<_, PaymentMethod>(
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
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching payment method: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch payment method: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Payment method {} not found", id),
            )),
        )
    })?;

    // TODO: Check if payment method has pending charges before deletion

    // TODO: Integrate with Tilled API to detach payment method

    // Soft delete the payment method
    sqlx::query(
        r#"
        UPDATE ar_payment_methods
        SET deleted_at = NOW(), is_default = FALSE, updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to delete payment method: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to delete payment method: {}", e),
            )),
        )
    })?;

    // If this was the default, clear customer fast-path
    if payment_method.is_default {
        sqlx::query(
            r#"
            UPDATE ar_customers
            SET default_payment_method_id = NULL, payment_method_type = NULL, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(payment_method.ar_customer_id)
        .execute(&db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to clear default payment method from customer: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    format!("Failed to update customer: {}", e),
                )),
            )
        })?;
    }

    tracing::info!("Deleted payment method {}", id);

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/ar/payment-methods/:id/set-default - Set payment method as default
async fn set_default_payment_method(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<PaymentMethod>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    // Verify payment method exists, belongs to app, and is active
    let payment_method = sqlx::query_as::<_, PaymentMethod>(
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
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching payment method: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch payment method: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Payment method {} not found", id),
            )),
        )
    })?;

    // Verify payment method is active
    if payment_method.status != "active" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                format!(
                    "Cannot set payment method with status {} as default",
                    payment_method.status
                ),
            )),
        ));
    }

    // Validate card expiration if it's a card
    if payment_method.payment_type == "card" {
        if let (Some(exp_month), Some(exp_year)) =
            (payment_method.exp_month, payment_method.exp_year)
        {
            let now = chrono::Utc::now();
            let current_year = now.year();
            let current_month = now.month() as i32;

            if exp_year < current_year || (exp_year == current_year && exp_month < current_month) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new(
                        "validation_error",
                        "Card is expired and cannot be set as default",
                    )),
                ));
            }
        }
    }

    // Use a transaction to atomically update defaults
    let mut tx = db.begin().await.map_err(|e| {
        tracing::error!("Failed to begin transaction: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to begin transaction",
            )),
        )
    })?;

    // Clear all other defaults for this customer
    sqlx::query(
        r#"
        UPDATE ar_payment_methods
        SET is_default = FALSE, updated_at = NOW()
        WHERE ar_customer_id = $1 AND app_id = $2
        "#,
    )
    .bind(payment_method.ar_customer_id)
    .bind(app_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Failed to clear default flags: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to clear default flags",
            )),
        )
    })?;

    // Set this payment method as default
    let updated_pm = sqlx::query_as::<_, PaymentMethod>(
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
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Failed to set default flag: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to set default flag",
            )),
        )
    })?;

    // Update customer fast-path
    sqlx::query(
        r#"
        UPDATE ar_customers
        SET default_payment_method_id = $1, payment_method_type = $2, updated_at = NOW()
        WHERE id = $3
        "#,
    )
    .bind(&payment_method.tilled_payment_method_id)
    .bind(&payment_method.payment_type)
    .bind(payment_method.ar_customer_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update customer default: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to update customer default",
            )),
        )
    })?;

    // Commit transaction
    tx.commit().await.map_err(|e| {
        tracing::error!("Failed to commit transaction: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to commit transaction",
            )),
        )
    })?;

    tracing::info!(
        "Set payment method {} as default for customer {}",
        id,
        payment_method.ar_customer_id
    );

    Ok(Json(updated_pm))
}

// ============================================================================
// WEBHOOK HANDLERS
// ============================================================================

/// Verify Tilled webhook signature
/// Tilled signs webhooks with HMAC-SHA256
fn verify_tilled_signature(
    payload: &[u8],
    signature_header: Option<&str>,
    secret: &str,
) -> Result<(), String> {
    let signature = signature_header.ok_or_else(|| "Missing signature header".to_string())?;

    // Tilled sends signature in format: "t=timestamp,v1=signature"
    let sig_parts: Vec<&str> = signature.split(',').collect();
    let mut timestamp = "";
    let mut sig_value = "";

    for part in sig_parts {
        if let Some(value) = part.strip_prefix("t=") {
            timestamp = value;
        } else if let Some(value) = part.strip_prefix("v1=") {
            sig_value = value;
        }
    }

    if timestamp.is_empty() || sig_value.is_empty() {
        return Err("Invalid signature format".to_string());
    }

    // Construct signed payload: timestamp.payload
    let signed_payload = format!("{}.{}", timestamp, String::from_utf8_lossy(payload));

    // Compute expected signature
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("Invalid secret: {}", e))?;
    mac.update(signed_payload.as_bytes());
    let expected_sig = hex::encode(mac.finalize().into_bytes());

    // Compare signatures (constant-time comparison)
    if expected_sig != sig_value {
        return Err("Signature verification failed".to_string());
    }

    Ok(())
}

/// Process webhook event based on type
async fn process_webhook_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    tracing::info!("Processing webhook event: {}", event.event_type);

    match event.event_type.as_str() {
        // Customer events
        "customer.created" | "customer.updated" => {
            process_customer_event(db, app_id, event).await?;
        }
        // Payment intent events
        "payment_intent.succeeded" | "payment_intent.failed" => {
            process_payment_intent_event(db, app_id, event).await?;
        }
        // Payment method events
        "payment_method.attached" | "payment_method.detached" => {
            process_payment_method_event(db, app_id, event).await?;
        }
        // Subscription events
        "subscription.created" | "subscription.updated" | "subscription.canceled" => {
            process_subscription_event(db, app_id, event).await?;
        }
        // Charge events
        "charge.succeeded" | "charge.failed" | "charge.refunded" => {
            process_charge_event(db, app_id, event).await?;
        }
        // Invoice events
        "invoice.created" | "invoice.payment_succeeded" | "invoice.payment_failed" => {
            process_invoice_event(db, app_id, event).await?;
        }
        _ => {
            tracing::warn!("Unhandled webhook event type: {}", event.event_type);
        }
    }

    Ok(())
}

/// Process customer webhook events
async fn process_customer_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let customer_data = &event.data;
    let tilled_customer_id = customer_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing customer ID in webhook data".to_string())?;

    // Update or create customer record
    let email = customer_data
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let name = customer_data.get("name").and_then(|v| v.as_str());

    sqlx::query(
        r#"
        INSERT INTO ar_customers (
            app_id, tilled_customer_id, email, name, status, metadata,
            retry_attempt_count, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, 'active', $5, 0, NOW(), NOW())
        ON CONFLICT (tilled_customer_id, app_id)
        DO UPDATE SET
            email = EXCLUDED.email,
            name = EXCLUDED.name,
            metadata = EXCLUDED.metadata,
            updated_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind(tilled_customer_id)
    .bind(email)
    .bind(name)
    .bind(&event.data)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update customer: {}", e))?;

    tracing::info!("Processed customer event for {}", tilled_customer_id);
    Ok(())
}

/// Process payment intent webhook events
async fn process_payment_intent_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let payment_intent_data = &event.data;
    let tilled_charge_id = payment_intent_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing payment intent ID".to_string())?;

    let status = if event.event_type == "payment_intent.succeeded" {
        "succeeded"
    } else {
        "failed"
    };

    // Update charge status
    sqlx::query(
        r#"
        UPDATE ar_charges
        SET status = $1, metadata = $2, updated_at = NOW()
        WHERE tilled_charge_id = $3 AND app_id = $4
        "#,
    )
    .bind(status)
    .bind(&event.data)
    .bind(tilled_charge_id)
    .bind(app_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update charge: {}", e))?;

    tracing::info!("Processed payment intent event for {}", tilled_charge_id);
    Ok(())
}

/// Process payment method webhook events
async fn process_payment_method_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let pm_data = &event.data;
    let tilled_pm_id = pm_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing payment method ID".to_string())?;

    if event.event_type == "payment_method.detached" {
        // Soft delete payment method
        sqlx::query(
            r#"
            UPDATE ar_payment_methods
            SET status = 'inactive', deleted_at = NOW(), updated_at = NOW()
            WHERE tilled_payment_method_id = $1 AND app_id = $2
            "#,
        )
        .bind(tilled_pm_id)
        .bind(app_id)
        .execute(db)
        .await
        .map_err(|e| format!("Failed to delete payment method: {}", e))?;
    }

    tracing::info!("Processed payment method event for {}", tilled_pm_id);
    Ok(())
}

/// Process subscription webhook events
async fn process_subscription_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let sub_data = &event.data;
    let tilled_sub_id = sub_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing subscription ID".to_string())?;

    let status = match event.event_type.as_str() {
        "subscription.created" => "active",
        "subscription.updated" => sub_data.get("status").and_then(|v| v.as_str()).unwrap_or("active"),
        "subscription.canceled" => "canceled",
        _ => "active",
    };

    // Update subscription status
    sqlx::query(
        r#"
        UPDATE ar_subscriptions
        SET status = $1, metadata = $2, updated_at = NOW()
        WHERE tilled_subscription_id = $3 AND app_id = $4
        "#,
    )
    .bind(status)
    .bind(&event.data)
    .bind(tilled_sub_id)
    .bind(app_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update subscription: {}", e))?;

    tracing::info!("Processed subscription event for {}", tilled_sub_id);
    Ok(())
}

/// Process charge webhook events
async fn process_charge_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let charge_data = &event.data;
    let tilled_charge_id = charge_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing charge ID".to_string())?;

    let status = match event.event_type.as_str() {
        "charge.succeeded" => "succeeded",
        "charge.failed" => "failed",
        "charge.refunded" => "refunded",
        _ => "pending",
    };

    sqlx::query(
        r#"
        UPDATE ar_charges
        SET status = $1, metadata = $2, updated_at = NOW()
        WHERE tilled_charge_id = $3 AND app_id = $4
        "#,
    )
    .bind(status)
    .bind(&event.data)
    .bind(tilled_charge_id)
    .bind(app_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update charge: {}", e))?;

    tracing::info!("Processed charge event for {}", tilled_charge_id);
    Ok(())
}

/// Process invoice webhook events
async fn process_invoice_event(
    db: &PgPool,
    app_id: &str,
    event: &TilledWebhookEvent,
) -> Result<(), String> {
    let invoice_data = &event.data;
    let tilled_invoice_id = invoice_data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing invoice ID".to_string())?;

    let status = match event.event_type.as_str() {
        "invoice.created" => "open",
        "invoice.payment_succeeded" => "paid",
        "invoice.payment_failed" => "unpaid",
        _ => "open",
    };

    sqlx::query(
        r#"
        UPDATE ar_invoices
        SET status = $1, metadata = $2, updated_at = NOW()
        WHERE tilled_invoice_id = $3 AND app_id = $4
        "#,
    )
    .bind(status)
    .bind(&event.data)
    .bind(tilled_invoice_id)
    .bind(app_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to update invoice: {}", e))?;

    tracing::info!("Processed invoice event for {}", tilled_invoice_id);
    Ok(())
}

/// POST /api/ar/webhooks/tilled - Receive Tilled webhook
async fn receive_tilled_webhook(
    State(db): State<PgPool>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // Extract app_id from headers or use default
    // TODO: Extract from auth middleware when available
    let app_id = "test-app";

    // Get webhook secret from environment (use test secret in test mode)
    let webhook_secret = std::env::var("TILLED_WEBHOOK_SECRET_TRASHTECH")
        .or_else(|_| std::env::var("TILLED_WEBHOOK_SECRET"))
        .unwrap_or_else(|_| "test-secret".to_string());

    // Skip signature verification in test mode
    if webhook_secret != "test-secret" {
        // Verify signature in production mode
        let signature = headers
            .get("tilled-signature")
            .or_else(|| headers.get("x-tilled-signature"))
            .and_then(|v| v.to_str().ok());

        if let Err(e) = verify_tilled_signature(&body, signature, &webhook_secret) {
            tracing::warn!("Webhook signature verification failed: {}", e);
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse::new("signature_error", e)),
            ));
        }
    }

    // Parse webhook event
    let event: TilledWebhookEvent = serde_json::from_slice(&body).map_err(|e| {
        tracing::error!("Failed to parse webhook event: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "parse_error",
                format!("Failed to parse webhook: {}", e),
            )),
        )
    })?;

    tracing::info!(
        "Received webhook event: {} (id: {})",
        event.event_type,
        event.id
    );

    // Check for duplicate event (idempotency)
    let existing = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT id FROM ar_webhooks
        WHERE event_id = $1 AND app_id = $2
        "#,
    )
    .bind(&event.id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to check for duplicate webhook: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to check idempotency",
            )),
        )
    })?;

    if existing.is_some() {
        tracing::info!("Webhook event {} already processed (idempotent)", event.id);
        // Return 200 to prevent Tilled retries
        return Ok(StatusCode::OK);
    }

    // Store webhook in database (status: received)
    let webhook_id = sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_webhooks (
            app_id, event_id, event_type, status, payload, attempt_count, received_at
        )
        VALUES ($1, $2, $3, 'received', $4, 1, NOW())
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(&event.id)
    .bind(&event.event_type)
    .bind(serde_json::to_value(&event).unwrap())
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to store webhook: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to store webhook",
            )),
        )
    })?;

    // Process event asynchronously (don't block webhook response)
    // Update status to processing
    sqlx::query(
        r#"
        UPDATE ar_webhooks
        SET status = 'processing', last_attempt_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(webhook_id)
    .execute(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update webhook status: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to update webhook status",
            )),
        )
    })?;

    // Process the event
    match process_webhook_event(&db, app_id, &event).await {
        Ok(_) => {
            // Mark as processed
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'processed', processed_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(webhook_id)
            .execute(&db)
            .await
            .ok();

            tracing::info!("Successfully processed webhook event {}", event.id);
        }
        Err(e) => {
            // Mark as failed
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'failed', error = $1, error_code = 'processing_error'
                WHERE id = $2
                "#,
            )
            .bind(&e)
            .bind(webhook_id)
            .execute(&db)
            .await
            .ok();

            tracing::error!("Failed to process webhook event {}: {}", event.id, e);
        }
    }

    // Always return 200 to prevent Tilled retries
    // Errors are stored in the database for manual investigation
    Ok(StatusCode::OK)
}

/// GET /api/ar/webhooks - List webhooks (admin)
async fn list_webhooks(
    State(db): State<PgPool>,
    Query(query): Query<ListWebhooksQuery>,
) -> Result<Json<Vec<Webhook>>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Add auth middleware to verify admin access
    let app_id = "test-app";

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0);

    let mut sql = String::from(
        r#"
        SELECT
            id, app_id, event_id, event_type, status, error, payload,
            attempt_count, last_attempt_at, next_attempt_at, dead_at,
            error_code, received_at, processed_at
        FROM ar_webhooks
        WHERE app_id = $1
        "#,
    );

    let mut param_count = 1;

    if query.event_type.is_some() {
        param_count += 1;
        sql.push_str(&format!(" AND event_type = ${}", param_count));
    }

    if query.status.is_some() {
        param_count += 1;
        sql.push_str(&format!(" AND status = ${}::ar_webhooks_status", param_count));
    }

    sql.push_str(" ORDER BY received_at DESC LIMIT $");
    param_count += 1;
    sql.push_str(&param_count.to_string());
    sql.push_str(" OFFSET $");
    param_count += 1;
    sql.push_str(&param_count.to_string());

    let mut query_builder = sqlx::query_as::<_, Webhook>(&sql).bind(app_id);

    if let Some(event_type) = &query.event_type {
        query_builder = query_builder.bind(event_type);
    }

    if let Some(status) = &query.status {
        query_builder = query_builder.bind(status);
    }

    query_builder = query_builder.bind(limit).bind(offset);

    let webhooks = query_builder.fetch_all(&db).await.map_err(|e| {
        tracing::error!("Failed to list webhooks: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to list webhooks",
            )),
        )
    })?;

    Ok(Json(webhooks))
}

/// GET /api/ar/webhooks/:id - Get webhook details
async fn get_webhook(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<Webhook>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Add auth middleware to verify admin access
    let app_id = "test-app";

    let webhook = sqlx::query_as::<_, Webhook>(
        r#"
        SELECT
            id, app_id, event_id, event_type, status, error, payload,
            attempt_count, last_attempt_at, next_attempt_at, dead_at,
            error_code, received_at, processed_at
        FROM ar_webhooks
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch webhook: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch webhook",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("not_found", "Webhook not found")),
        )
    })?;

    Ok(Json(webhook))
}

/// POST /api/ar/webhooks/:id/replay - Replay a webhook
async fn replay_webhook(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
    Json(req): Json<ReplayWebhookRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Add auth middleware to verify admin access
    let app_id = "test-app";

    // Fetch webhook
    let webhook = sqlx::query_as::<_, Webhook>(
        r#"
        SELECT
            id, app_id, event_id, event_type, status, error, payload,
            attempt_count, last_attempt_at, next_attempt_at, dead_at,
            error_code, received_at, processed_at
        FROM ar_webhooks
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch webhook: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch webhook",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("not_found", "Webhook not found")),
        )
    })?;

    // Check if replay is allowed
    let force = req.force.unwrap_or(false);
    if webhook.status != WebhookStatus::Failed && !force {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_status",
                "Can only replay failed webhooks (use force=true to override)",
            )),
        ));
    }

    // Parse payload
    let event: TilledWebhookEvent = serde_json::from_value(
        webhook.payload.ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new("invalid_webhook", "Webhook has no payload")),
            )
        })?,
    )
    .map_err(|e| {
        tracing::error!("Failed to parse webhook payload: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "parse_error",
                "Failed to parse webhook payload",
            )),
        )
    })?;

    // Update status to processing
    sqlx::query(
        r#"
        UPDATE ar_webhooks
        SET status = 'processing', last_attempt_at = NOW(), attempt_count = attempt_count + 1
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update webhook status: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to update webhook status",
            )),
        )
    })?;

    // Process the event
    match process_webhook_event(&db, app_id, &event).await {
        Ok(_) => {
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'processed', processed_at = NOW(), error = NULL, error_code = NULL
                WHERE id = $1
                "#,
            )
            .bind(id)
            .execute(&db)
            .await
            .ok();

            tracing::info!("Successfully replayed webhook {}", id);
            Ok(StatusCode::OK)
        }
        Err(e) => {
            sqlx::query(
                r#"
                UPDATE ar_webhooks
                SET status = 'failed', error = $1, error_code = 'processing_error'
                WHERE id = $2
                "#,
            )
            .bind(&e)
            .bind(id)
            .execute(&db)
            .await
            .ok();

            tracing::error!("Failed to replay webhook {}: {}", id, e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("processing_error", e)),
            ))
        }
    }
}

// ============================================================================
// EVENT LOG ENDPOINTS
// ============================================================================

/// GET /api/ar/events - List events with filtering
async fn list_events(
    State(db): State<PgPool>,
    Query(query): Query<ListEventsQuery>,
) -> Result<Json<Vec<Event>>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app";

    let limit = query.limit.unwrap_or(100).min(1000);
    let offset = query.offset.unwrap_or(0);

    // Build dynamic query based on filters
    let mut sql = String::from(
        "SELECT id, app_id, event_type, source, entity_type, entity_id, payload, created_at FROM ar_events WHERE app_id = $1",
    );
    let mut param_count = 1;
    let mut conditions = Vec::new();

    // Add filters
    if query.entity_id.is_some() {
        param_count += 1;
        conditions.push(format!("entity_id = ${}", param_count));
    }
    if query.entity_type.is_some() {
        param_count += 1;
        conditions.push(format!("entity_type = ${}", param_count));
    }
    if query.event_type.is_some() {
        param_count += 1;
        conditions.push(format!("event_type = ${}", param_count));
    }
    if query.source.is_some() {
        param_count += 1;
        conditions.push(format!("source = ${}", param_count));
    }
    if query.start.is_some() {
        param_count += 1;
        conditions.push(format!("created_at >= ${}", param_count));
    }
    if query.end.is_some() {
        param_count += 1;
        conditions.push(format!("created_at <= ${}", param_count));
    }

    if !conditions.is_empty() {
        sql.push_str(" AND ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(" ORDER BY created_at DESC");
    param_count += 1;
    sql.push_str(&format!(" LIMIT ${}", param_count));
    param_count += 1;
    sql.push_str(&format!(" OFFSET ${}", param_count));

    // Build query with parameters
    let mut q = sqlx::query_as::<_, Event>(&sql).bind(app_id);

    if let Some(ref entity_id) = query.entity_id {
        q = q.bind(entity_id);
    }
    if let Some(ref entity_type) = query.entity_type {
        q = q.bind(entity_type);
    }
    if let Some(ref event_type) = query.event_type {
        q = q.bind(event_type);
    }
    if let Some(ref source) = query.source {
        q = q.bind(source);
    }
    if let Some(start) = query.start {
        q = q.bind(start);
    }
    if let Some(end) = query.end {
        q = q.bind(end);
    }

    q = q.bind(limit).bind(offset);

    let events = q.fetch_all(&db).await.map_err(|e| {
        tracing::error!("Failed to list events: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("database_error", e.to_string())),
        )
    })?;

    Ok(Json(events))
}

/// GET /api/ar/events/{id} - Get a single event by ID
async fn get_event(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<Event>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app";

    let event = sqlx::query_as::<_, Event>(
        r#"
        SELECT id, app_id, event_type, source, entity_type, entity_id, payload, created_at
        FROM ar_events
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch event {}: {}", id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("database_error", e.to_string())),
        )
    })?;

    match event {
        Some(e) => Ok(Json(e)),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Event {} not found", id),
            )),
        )),
    }
}
