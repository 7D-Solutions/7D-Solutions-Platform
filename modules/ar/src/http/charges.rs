use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::domain::{charges, customers};
use crate::models::{
    ApiError, CaptureChargeRequest, Charge, CreateChargeRequest, ListChargesQuery,
    PaginatedResponse,
};
use crate::tilled::TilledClient;

/// POST /api/ar/charges - Create a new charge
#[utoipa::path(post, path = "/api/ar/charges", tag = "Charges",
    request_body = CreateChargeRequest,
    responses(
        (status = 201, description = "Charge created", body = Charge),
        (status = 400, description = "Validation error", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn create_charge(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateChargeRequest>,
) -> Result<(StatusCode, Json<Charge>), ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    // Validate required fields
    if req.amount_cents <= 0 {
        return Err(ApiError::bad_request("amount_cents must be greater than 0"));
    }

    if req.reason.is_empty() {
        return Err(ApiError::bad_request("reason is required"));
    }

    if req.reference_id.is_empty() {
        return Err(ApiError::bad_request("reference_id is required"));
    }

    // Verify customer exists and belongs to app
    let customer = customers::fetch_customer(&db, req.ar_customer_id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error fetching customer");
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| ApiError::not_found(format!("Customer {} not found", req.ar_customer_id)))?;

    // Ensure default payment method exists
    if customer.default_payment_method_id.is_none() {
        return Err(ApiError::conflict("No default payment method on file"));
    }

    // Check for duplicate reference_id
    let existing_charge = charges::find_by_reference_id(&db, &app_id, &req.reference_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error checking duplicate charge");
            ApiError::internal("Internal database error")
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

    let charge = charges::insert_charge(
        &db,
        &app_id,
        req.ar_customer_id,
        req.amount_cents,
        &currency,
        &charge_type,
        &req.reason,
        &req.reference_id,
        req.service_date,
        req.note,
        req.metadata,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to create charge");
        ApiError::internal("Internal database error")
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
#[utoipa::path(get, path = "/api/ar/charges/{id}", tag = "Charges",
    params(("id" = i32, Path, description = "Charge ID")),
    responses(
        (status = 200, description = "Charge found", body = Charge),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_charge(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Charge>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let charge = charges::fetch_with_tenant(&db, id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error fetching charge");
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| ApiError::not_found(format!("Charge {} not found", id)))?;

    Ok(Json(charge))
}

/// GET /api/ar/charges - List charges (with optional filtering)
#[utoipa::path(get, path = "/api/ar/charges", tag = "Charges",
    params(ListChargesQuery),
    responses(
        (status = 200, description = "Paginated charges", body = PaginatedResponse<Charge>),
    ),
    security(("bearer" = [])))]
pub async fn list_charges(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListChargesQuery>,
) -> Result<Json<PaginatedResponse<Charge>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    let total_items = charges::count_charges(
        &db,
        &app_id,
        query.customer_id,
        query.invoice_id,
        query.status.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Database error counting charges");
        ApiError::internal("Internal database error")
    })?;

    let charge_list = charges::list_charges(
        &db,
        &app_id,
        query.customer_id,
        query.invoice_id,
        query.status.as_deref(),
        limit,
        offset,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Database error listing charges");
        ApiError::internal("Internal database error")
    })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(
        charge_list,
        page,
        limit as i64,
        total_items,
    )))
}

/// POST /api/ar/charges/:id/capture - Capture an authorized charge
#[utoipa::path(post, path = "/api/ar/charges/{id}/capture", tag = "Charges",
    params(("id" = i32, Path, description = "Charge ID")),
    request_body = CaptureChargeRequest,
    responses(
        (status = 200, description = "Charge captured", body = Charge),
        (status = 400, description = "Invalid charge state", body = platform_http_contracts::ApiError),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn capture_charge(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<CaptureChargeRequest>,
) -> Result<Json<Charge>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let existing = charges::fetch_with_tenant(&db, id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error fetching charge");
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| ApiError::not_found(format!("Charge {} not found", id)))?;

    if existing.status != "authorized" {
        return Err(ApiError::bad_request(format!(
            "Cannot capture charge with status {}",
            existing.status
        )));
    }

    let capture_amount = req.amount_cents.unwrap_or(existing.amount_cents);

    if capture_amount <= 0 {
        return Err(ApiError::bad_request("Capture amount must be positive"));
    }

    let tilled_charge_id = existing.tilled_charge_id.as_deref().ok_or_else(|| {
        ApiError::conflict(
            "Charge has no provider ID — cannot capture until provider confirms authorization",
        )
    })?;

    let client = TilledClient::from_env(&app_id).map_err(|e| {
        tracing::error!(error = %e, "Failed to create Tilled client");
        ApiError::internal("Internal database error")
    })?;

    let capture_amount_i64 = capture_amount;

    match client
        .capture_payment_intent(tilled_charge_id, Some(capture_amount_i64))
        .await
    {
        Ok(pi) => {
            let new_status = if pi.status == "succeeded" {
                "succeeded"
            } else {
                &pi.status
            };

            let charge = charges::update_after_capture(&db, id, new_status, capture_amount)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "Failed to update charge after capture");
                    ApiError::internal("Internal database error")
                })?;

            tracing::info!("Captured charge {} (amount: {})", id, capture_amount);
            Ok(Json(charge))
        }
        Err(e) => {
            tracing::error!(id = %id, error = %e, "Tilled capture failed for charge");
            Err(ApiError::new(
                502,
                "provider_error",
                format!("Payment provider capture failed: {}", e),
            ))
        }
    }
}
