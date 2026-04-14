use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::domain::customers;
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
    let existing = customers::check_email_exists(&db, &app_id, email)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to check duplicate email");
            ApiError::internal("Internal database error")
        })?;

    if existing.is_some() {
        return Err(ApiError::conflict(
            "A customer with this email already exists",
        ));
    }

    let customer = customers::insert_customer(
        &db,
        &app_id,
        req.external_customer_id,
        email,
        req.name,
        req.metadata,
        req.party_id,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to create customer");

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

    let customer = customers::fetch_customer(&db, id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error fetching customer");
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| ApiError::not_found(format!("Customer {} not found", id)))?;

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

    let total_items =
        customers::count_customers(&db, &app_id, query.external_customer_id.as_deref())
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Database error counting customers");
                ApiError::internal("Internal database error")
            })?;

    let customer_list =
        customers::list_customers(&db, &app_id, query.external_customer_id, limit, offset)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Database error listing customers");
                ApiError::internal("Internal database error")
            })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(
        customer_list,
        page,
        limit as i64,
        total_items,
    )))
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
    let existing = customers::fetch_customer(&db, id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error fetching customer");
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| ApiError::not_found(format!("Customer {} not found", id)))?;

    // Validate at least one field is being updated
    if req.email.is_none() && req.name.is_none() && req.metadata.is_none() && req.party_id.is_none()
    {
        return Err(ApiError::bad_request("No valid fields to update"));
    }

    let email = req.email.unwrap_or(existing.email);
    let name = req.name.or(existing.name);
    let metadata = req.metadata.or(existing.metadata);
    let party_id = if req.party_id.is_some() {
        req.party_id
    } else {
        existing.party_id
    };

    let customer = customers::update_customer(&db, id, &email, name, metadata, party_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to update customer");
            ApiError::internal("Internal database error")
        })?;

    Ok(Json(customer))
}
