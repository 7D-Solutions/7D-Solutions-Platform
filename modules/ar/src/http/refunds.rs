use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{ApiError, Charge, CreateRefundRequest, ListRefundsQuery, PaginatedResponse, Refund};
use crate::tilled::types::checked_i32_to_i64;
use crate::tilled::TilledClient;

/// POST /api/ar/refunds - Create a refund for a charge
#[utoipa::path(post, path = "/api/ar/refunds", tag = "Refunds",
    request_body = CreateRefundRequest,
    responses(
        (status = 201, description = "Refund created", body = Refund),
        (status = 400, description = "Validation error", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn create_refund(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateRefundRequest>,
) -> Result<(StatusCode, Json<Refund>), ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    // Validate required fields
    if req.amount_cents <= 0 {
        return Err(ApiError::bad_request("amount_cents must be greater than 0"));
    }

    if req.reference_id.trim().is_empty() {
        return Err(ApiError::bad_request("reference_id is required"));
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
    .bind(&app_id)
    .bind(&req.reference_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error checking duplicate refund: {:?}", e);
        ApiError::internal("Internal database error")
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
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching charge: {:?}", e);
        ApiError::internal("Internal database error")
    })?
    .ok_or_else(|| ApiError::not_found("Charge not found"))?;

    // Ensure charge has been settled in processor
    if charge.tilled_charge_id.is_none() {
        return Err(ApiError::conflict("Charge not settled in processor"));
    }

    // Validate refund amount does not exceed charge amount
    if req.amount_cents > charge.amount_cents {
        return Err(ApiError::bad_request(format!(
            "Refund amount ({}) exceeds charge amount ({})",
            req.amount_cents, charge.amount_cents
        )));
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
    .bind(&app_id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error calculating refunded amount: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    let total_refunded = total_refunded.unwrap_or(0) as i32;
    let remaining_refundable = charge.amount_cents - total_refunded;

    if req.amount_cents > remaining_refundable {
        return Err(ApiError::bad_request(format!(
            "Refund amount ({}) exceeds remaining refundable amount ({}). Total already refunded: {}",
            req.amount_cents, remaining_refundable, total_refunded
        )));
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
    .bind(&app_id)
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
        ApiError::internal("Internal database error")
    })?;

    // Call Tilled API to create the refund
    let payment_intent_id = charge.tilled_charge_id.ok_or_else(|| {
        ApiError::internal("Internal database error")
    })?;
    let client = TilledClient::from_env(&app_id).map_err(|e| {
        tracing::error!("Failed to create Tilled client: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    let amount_i64 = checked_i32_to_i64(req.amount_cents);
    let tilled_metadata = req.metadata.as_ref().and_then(|m| {
        serde_json::from_value::<std::collections::HashMap<String, String>>(m.clone()).ok()
    });

    match client
        .create_refund(
            payment_intent_id,
            amount_i64,
            req.currency.clone(),
            req.reason.clone(),
            tilled_metadata,
        )
        .await
    {
        Ok(tilled_refund) => {
            let refund = sqlx::query_as::<_, Refund>(
                r#"
                UPDATE ar_refunds
                SET status = $1, tilled_refund_id = $2, updated_at = NOW()
                WHERE id = $3
                RETURNING
                    id, app_id, ar_customer_id, charge_id, tilled_refund_id, tilled_charge_id,
                    status, amount_cents, currency, reason, reference_id, note, metadata,
                    failure_code, failure_message, created_at, updated_at
                "#,
            )
            .bind(&tilled_refund.status)
            .bind(&tilled_refund.id)
            .bind(refund.id)
            .fetch_one(&db)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update refund after provider call: {:?}", e);
                ApiError::internal("Internal database error")
            })?;

            tracing::info!(
                "Created refund {} for charge {} (amount: {}, tilled_id: {})",
                refund.id,
                req.charge_id,
                req.amount_cents,
                tilled_refund.id,
            );

            Ok((StatusCode::CREATED, Json(refund)))
        }
        Err(e) => {
            tracing::error!("Tilled refund failed for charge {}: {:?}", req.charge_id, e);
            // Keep refund as 'pending' — do not advance status on failure
            Err(ApiError::new(
                502,
                "provider_error",
                format!("Payment provider refund failed: {}", e),
            ))
        }
    }
}

/// GET /api/ar/refunds/{id} - Get a specific refund
#[utoipa::path(get, path = "/api/ar/refunds/{id}", tag = "Refunds",
    params(("id" = i32, Path, description = "Refund ID")),
    responses(
        (status = 200, description = "Refund found", body = Refund),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_refund(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Refund>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

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
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching refund: {:?}", e);
        ApiError::internal("Internal database error")
    })?
    .ok_or_else(|| {
        ApiError::not_found(format!("Refund {} not found", id))
    })?;

    Ok(Json(refund))
}

/// GET /api/ar/refunds - List refunds with optional filters
#[utoipa::path(get, path = "/api/ar/refunds", tag = "Refunds",
    params(ListRefundsQuery),
    responses(
        (status = 200, description = "Paginated refunds", body = PaginatedResponse<Refund>),
    ),
    security(("bearer" = [])))]
pub async fn list_refunds(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListRefundsQuery>,
) -> Result<Json<PaginatedResponse<Refund>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

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

    let mut query_builder = sqlx::query_as::<_, Refund>(&sql).bind(&app_id);

    if let Some(charge_id) = query.charge_id {
        query_builder = query_builder.bind(charge_id);
    }
    if let Some(customer_id) = query.customer_id {
        query_builder = query_builder.bind(customer_id);
    }
    if let Some(ref status) = query.status {
        query_builder = query_builder.bind(status);
    }

    let refunds = query_builder
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
        .await
        .map_err(|e| {
            tracing::error!("Database error listing refunds: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    // Count total matching rows (same dynamic filters)
    let mut count_sql = String::from("SELECT COUNT(*) FROM ar_refunds WHERE app_id = $1");
    let mut count_idx = 2;
    if query.charge_id.is_some() {
        count_sql.push_str(&format!(" AND charge_id = ${count_idx}"));
        count_idx += 1;
    }
    if query.customer_id.is_some() {
        count_sql.push_str(&format!(" AND ar_customer_id = ${count_idx}"));
        count_idx += 1;
    }
    if query.status.is_some() {
        count_sql.push_str(&format!(" AND status = ${count_idx}"));
    }
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(&app_id);
    if let Some(cid) = query.charge_id {
        count_q = count_q.bind(cid);
    }
    if let Some(cust_id) = query.customer_id {
        count_q = count_q.bind(cust_id);
    }
    if let Some(ref st) = query.status {
        count_q = count_q.bind(st);
    }
    let total_items = count_q.fetch_one(&db).await.map_err(|e| {
        tracing::error!("Database error counting refunds: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(refunds, page, limit as i64, total_items)))
}
