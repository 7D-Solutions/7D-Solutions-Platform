use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use sqlx::PgPool;

use crate::idempotency::log_event_async;
use crate::models::{
    CreateCustomerRequest, Customer, ErrorResponse, ListCustomersQuery, UpdateCustomerRequest,
};

/// POST /api/ar/customers - Create a new customer
pub async fn create_customer(
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
pub async fn get_customer(
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
pub async fn list_customers(
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
pub async fn update_customer(
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
