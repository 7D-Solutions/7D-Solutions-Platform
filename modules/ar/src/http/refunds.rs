use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::domain::refunds;
use crate::models::{ApiError, CreateRefundRequest, ListRefundsQuery, PaginatedResponse, Refund};
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

    if req.amount_cents <= 0 {
        return Err(ApiError::bad_request("amount_cents must be greater than 0"));
    }

    if req.reference_id.trim().is_empty() {
        return Err(ApiError::bad_request("reference_id is required"));
    }

    // Check for duplicate reference_id (domain-level idempotency)
    let existing_refund = refunds::find_by_reference_id(&db, &app_id, &req.reference_id)
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
    let charge = refunds::fetch_charge_for_refund(&db, req.charge_id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error fetching charge: {:?}", e);
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| ApiError::not_found("Charge not found"))?;

    if charge.tilled_charge_id.is_none() {
        return Err(ApiError::conflict("Charge not settled in processor"));
    }

    if req.amount_cents > charge.amount_cents {
        return Err(ApiError::bad_request(format!(
            "Refund amount ({}) exceeds charge amount ({})",
            req.amount_cents, charge.amount_cents
        )));
    }

    let total_refunded = refunds::sum_refunded(&db, req.charge_id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error calculating refunded amount: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    let total_refunded = total_refunded.unwrap_or(0);
    let remaining_refundable = charge.amount_cents - total_refunded;

    if req.amount_cents > remaining_refundable {
        return Err(ApiError::bad_request(format!(
            "Refund amount ({}) exceeds remaining refundable amount ({}). Total already refunded: {}",
            req.amount_cents, remaining_refundable, total_refunded
        )));
    }

    let refund = refunds::insert_refund(
        &db,
        &app_id,
        charge.ar_customer_id,
        req.charge_id,
        &charge.tilled_charge_id,
        req.amount_cents,
        req.currency.as_deref().unwrap_or("usd"),
        &req.reason,
        &req.reference_id,
        &req.note,
        &req.metadata,
    )
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

    let amount_i64 = req.amount_cents;
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
            let refund = refunds::update_after_provider(&db, refund.id, &tilled_refund.status, &tilled_refund.id)
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

    let refund = refunds::fetch_by_id(&db, id, &app_id)
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

    let refund_list = refunds::list_refunds(
        &db,
        &app_id,
        query.charge_id,
        query.customer_id,
        query.status.as_deref(),
        limit,
        offset,
    )
    .await
    .map_err(|e| {
        tracing::error!("Database error listing refunds: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    let total_items = refunds::count_refunds(
        &db,
        &app_id,
        query.charge_id,
        query.customer_id,
        query.status.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!("Database error counting refunds: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(refund_list, page, limit as i64, total_items)))
}
