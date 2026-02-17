use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use sqlx::PgPool;

use crate::models::{Charge, CreateRefundRequest, ErrorResponse, ListRefundsQuery, Refund};

/// POST /api/ar/refunds - Create a refund for a charge
pub async fn create_refund(
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
pub async fn get_refund(
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
pub async fn list_refunds(
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
