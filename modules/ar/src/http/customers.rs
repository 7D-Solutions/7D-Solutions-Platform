use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::idempotency::log_event_async;
use crate::models::{
    ApiError, CreateCustomerRequest, Customer, ListCustomersQuery, PaginatedResponse,
    UpdateCustomerRequest,
};

/// POST /api/ar/customers - Create a new customer
#[utoipa::path(post, path = "/api/ar/customers", tag = "Customers",
    request_body = CreateCustomerRequest,
    responses(
        (status = 201, description = "Customer created", body = Customer),
        (status = 400, description = "Validation error", body = platform_http_contracts::ApiError),
        (status = 409, description = "Duplicate email", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn create_customer(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateCustomerRequest>,
) -> Result<(StatusCode, Json<Customer>), ApiError> {
    // Validate email
    let email = match &req.email {
        Some(e) if !e.trim().is_empty() => e.trim(),
        _ => {
            return Err(ApiError::bad_request("Email is required"));
        }
    };

    let app_id = super::tenant::extract_tenant(&claims)?;

    // Check for duplicate email
    let existing: Option<(i32,)> =
        sqlx::query_as("SELECT id FROM ar_customers WHERE app_id = $1 AND email = $2 LIMIT 1")
            .bind(&app_id)
            .bind(email)
            .fetch_optional(&db)
            .await
            .map_err(|e| {
                tracing::error!("Failed to check duplicate email: {:?}", e);
                ApiError::internal("Internal database error")
            })?;

    if existing.is_some() {
        return Err(ApiError::conflict("A customer with this email already exists"));
    }

    // Create customer in database (local-first, low-code pattern).
    // Status starts as 'pending_sync'; the provider webhook (customer.created)
    // will set tilled_customer_id and transition status to 'active'.
    let customer = sqlx::query_as::<_, Customer>(
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
    .bind(&app_id)
    .bind(req.external_customer_id)
    .bind(email)
    .bind(req.name)
    .bind(req.metadata)
    .bind(req.party_id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create customer: {:?}", e);

        // Check for unique constraint violation (duplicate email)
        if let sqlx::Error::Database(db_err) = &e {
            if db_err.code() == Some(std::borrow::Cow::Borrowed("23505")) {
                return ApiError::conflict("A customer with this email already exists");
            }
        }

        ApiError::internal("Internal database error")
    })?;

    // Log event asynchronously
    log_event_async(
        db.clone(),
        app_id,
        "customer.created".to_string(),
        "api".to_string(),
        Some("customer".to_string()),
        Some(customer.id.to_string()),
        Some(serde_json::to_value(&customer).unwrap_or_default()),
    );

    Ok((StatusCode::CREATED, Json(customer)))
}

/// GET /api/ar/customers/:id - Get customer by ID
#[utoipa::path(get, path = "/api/ar/customers/{id}", tag = "Customers",
    params(("id" = i32, Path, description = "Customer ID")),
    responses(
        (status = 200, description = "Customer found", body = Customer),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_customer(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Customer>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

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
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching customer: {:?}", e);
        ApiError::internal("Internal database error")
    })?
    .ok_or_else(|| {
        ApiError::not_found(format!("Customer {} not found", id))
    })?;

    Ok(Json(customer))
}

/// GET /api/ar/customers - List customers (with optional filtering)
#[utoipa::path(get, path = "/api/ar/customers", tag = "Customers",
    params(ListCustomersQuery),
    responses(
        (status = 200, description = "Paginated customers", body = PaginatedResponse<Customer>),
    ),
    security(("bearer" = [])))]
pub async fn list_customers(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListCustomersQuery>,
) -> Result<Json<PaginatedResponse<Customer>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Count total matching rows
    let mut count_sql = String::from("SELECT COUNT(*) FROM ar_customers WHERE app_id = $1");
    if query.external_customer_id.is_some() {
        count_sql.push_str(" AND external_customer_id = $2");
    }
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(&app_id);
    if let Some(ref ext_id) = query.external_customer_id {
        count_q = count_q.bind(ext_id);
    }
    let total_items = count_q.fetch_one(&db).await.map_err(|e| {
        tracing::error!("Database error counting customers: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    let customers = if let Some(external_id) = query.external_customer_id {
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
        .bind(&app_id)
        .bind(external_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
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
        .bind(&app_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
        .await
    }
    .map_err(|e| {
        tracing::error!("Database error listing customers: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(customers, page, limit as i64, total_items)))
}

/// PUT /api/ar/customers/:id - Update customer
#[utoipa::path(put, path = "/api/ar/customers/{id}", tag = "Customers",
    params(("id" = i32, Path, description = "Customer ID")),
    request_body = UpdateCustomerRequest,
    responses(
        (status = 200, description = "Customer updated", body = Customer),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn update_customer(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<UpdateCustomerRequest>,
) -> Result<Json<Customer>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

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
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching customer: {:?}", e);
        ApiError::internal("Internal database error")
    })?
    .ok_or_else(|| {
        ApiError::not_found(format!("Customer {} not found", id))
    })?;

    // Validate at least one field is being updated
    if req.email.is_none() && req.name.is_none() && req.metadata.is_none() && req.party_id.is_none()
    {
        return Err(ApiError::bad_request("No valid fields to update"));
    }

    // Build dynamic update query based on provided fields
    let email = req.email.unwrap_or(existing.email);
    let name = req.name.or(existing.name);
    let metadata = req.metadata.or(existing.metadata);
    let party_id = if req.party_id.is_some() {
        req.party_id
    } else {
        existing.party_id
    };

    // Update local fields immediately. Set update_source = 'local' to distinguish
    // from webhook-driven updates. If provider sync is needed, it happens via
    // the hosted flow and the webhook (customer.updated) reconciles.
    let customer = sqlx::query_as::<_, Customer>(
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
    .bind(&email)
    .bind(name)
    .bind(metadata)
    .bind(party_id)
    .bind(id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update customer: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    Ok(Json(customer))
}
