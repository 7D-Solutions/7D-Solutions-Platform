use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{
    ApiError, CaptureChargeRequest, Charge, CreateChargeRequest, Customer, ListChargesQuery,
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

    // Ensure default payment method exists
    if customer.default_payment_method_id.is_none() {
        return Err(ApiError::conflict("No default payment method on file"));
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
    .bind(&app_id)
    .bind(&req.reference_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error checking duplicate charge: {:?}", e);
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
    .bind(&app_id)
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
        ApiError::internal("Internal database error")
    })?;

    // Charge stays pending with NULL tilled_charge_id.
    // The provider webhook (payment_intent.succeeded / charge.succeeded)
    // will set the real provider ID and transition status.

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
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching charge: {:?}", e);
        ApiError::internal("Internal database error")
    })?
    .ok_or_else(|| {
        ApiError::not_found(format!("Charge {} not found", id))
    })?;

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

    // Count total matching rows
    let mut count_sql = String::from(
        "SELECT COUNT(*) FROM ar_charges ch \
         INNER JOIN ar_customers c ON ch.ar_customer_id = c.id \
         WHERE c.app_id = $1",
    );
    let mut bind_idx = 2;
    if query.customer_id.is_some() {
        count_sql.push_str(&format!(" AND ch.ar_customer_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if query.invoice_id.is_some() {
        count_sql.push_str(&format!(" AND ch.invoice_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if query.status.is_some() {
        count_sql.push_str(&format!(" AND ch.status = ${bind_idx}"));
    }
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(&app_id);
    if let Some(cid) = query.customer_id {
        count_q = count_q.bind(cid);
    }
    if let Some(iid) = query.invoice_id {
        count_q = count_q.bind(iid);
    }
    if let Some(ref st) = query.status {
        count_q = count_q.bind(st);
    }
    let total_items = count_q.fetch_one(&db).await.map_err(|e| {
        tracing::error!("Database error counting charges: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    // Fetch data with dynamic SQL
    let mut data_sql = String::from(
        r#"SELECT
            ch.id, ch.app_id, ch.tilled_charge_id, ch.invoice_id, ch.ar_customer_id, ch.subscription_id,
            ch.status, ch.amount_cents, ch.currency, ch.charge_type, ch.reason, ch.reference_id,
            ch.service_date, ch.note, ch.metadata, ch.failure_code, ch.failure_message,
            ch.product_type, ch.quantity, ch.service_frequency, ch.weight_amount, ch.location_reference,
            ch.created_at, ch.updated_at
        FROM ar_charges ch
        INNER JOIN ar_customers c ON ch.ar_customer_id = c.id
        WHERE c.app_id = $1"#,
    );
    let mut data_idx = 2;
    if query.customer_id.is_some() {
        data_sql.push_str(&format!(" AND ch.ar_customer_id = ${data_idx}"));
        data_idx += 1;
    }
    if query.invoice_id.is_some() {
        data_sql.push_str(&format!(" AND ch.invoice_id = ${data_idx}"));
        data_idx += 1;
    }
    if query.status.is_some() {
        data_sql.push_str(&format!(" AND ch.status = ${data_idx}"));
        data_idx += 1;
    }
    data_sql.push_str(&format!(
        " ORDER BY ch.created_at DESC LIMIT ${data_idx} OFFSET ${}",
        data_idx + 1
    ));

    let mut data_q = sqlx::query_as::<_, Charge>(&data_sql).bind(&app_id);
    if let Some(cid) = query.customer_id {
        data_q = data_q.bind(cid);
    }
    if let Some(iid) = query.invoice_id {
        data_q = data_q.bind(iid);
    }
    if let Some(ref st) = query.status {
        data_q = data_q.bind(st);
    }
    let charges = data_q
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
        .await
        .map_err(|e| {
            tracing::error!("Database error listing charges: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(charges, page, limit as i64, total_items)))
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
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching charge: {:?}", e);
        ApiError::internal("Internal database error")
    })?
    .ok_or_else(|| {
        ApiError::not_found(format!("Charge {} not found", id))
    })?;

    // Only authorized charges can be captured
    if existing.status != "authorized" {
        return Err(ApiError::bad_request(format!("Cannot capture charge with status {}", existing.status)));
    }

    // Use provided amount or existing amount
    let capture_amount = req.amount_cents.unwrap_or(existing.amount_cents);

    // Validate capture amount is positive
    if capture_amount <= 0 {
        return Err(ApiError::bad_request("Capture amount must be positive"));
    }

    // Require a provider ID — capture only works on charges that Tilled knows about
    let tilled_charge_id = existing.tilled_charge_id.as_deref().ok_or_else(|| {
        ApiError::conflict("Charge has no provider ID — cannot capture until provider confirms authorization")
    })?;

    let client = TilledClient::from_env(&app_id).map_err(|e| {
        tracing::error!("Failed to create Tilled client: {:?}", e);
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

            let charge = sqlx::query_as::<_, Charge>(
                r#"
                UPDATE ar_charges
                SET status = $1, amount_cents = $2, updated_at = NOW()
                WHERE id = $3
                RETURNING
                    id, app_id, tilled_charge_id, invoice_id, ar_customer_id, subscription_id,
                    status, amount_cents, currency, charge_type, reason, reference_id,
                    service_date, note, metadata, failure_code, failure_message,
                    product_type, quantity, service_frequency, weight_amount, location_reference,
                    created_at, updated_at
                "#,
            )
            .bind(new_status)
            .bind(capture_amount)
            .bind(id)
            .fetch_one(&db)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update charge after capture: {:?}", e);
                ApiError::internal("Internal database error")
            })?;

            tracing::info!("Captured charge {} (amount: {})", id, capture_amount);
            Ok(Json(charge))
        }
        Err(e) => {
            tracing::error!("Tilled capture failed for charge {}: {:?}", id, e);
            Err(ApiError::new(
                502,
                "provider_error",
                format!("Payment provider capture failed: {}", e),
            ))
        }
    }
}
