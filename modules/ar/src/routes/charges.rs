use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{
    CaptureChargeRequest, Charge, CreateChargeRequest, Customer, ErrorResponse, ListChargesQuery,
};

/// POST /api/ar/charges - Create a new charge
pub async fn create_charge(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateChargeRequest>,
) -> Result<(StatusCode, Json<Charge>), (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

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
    .bind(&app_id)
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
    .bind(&app_id)
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to create charge: {}", e),
            )),
        )
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
pub async fn get_charge(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Charge>, (StatusCode, Json<ErrorResponse>)> {
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
pub async fn list_charges(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListChargesQuery>,
) -> Result<Json<Vec<Charge>>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

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
            .bind(&app_id)
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
            .bind(&app_id)
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
            .bind(&app_id)
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
            .bind(&app_id)
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
            .bind(&app_id)
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
pub async fn capture_charge(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<CaptureChargeRequest>,
) -> Result<Json<Charge>, (StatusCode, Json<ErrorResponse>)> {
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

    // Validate capture amount is positive
    if capture_amount <= 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "Capture amount must be positive",
            )),
        ));
    }

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
