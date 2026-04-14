use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::domain::payment_methods;
use crate::models::{ApiError, ListPaymentMethodsQuery, PaginatedResponse, PaymentMethod};

/// GET /api/ar/payment-methods/:id - Get payment method by ID
#[utoipa::path(get, path = "/api/ar/payment-methods/{id}", tag = "Payment Methods",
    params(("id" = i32, Path, description = "Payment method ID")),
    responses(
        (status = 200, description = "Payment method found", body = PaymentMethod),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_payment_method(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<PaymentMethod>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let payment_method = payment_methods::fetch_with_tenant(&db, id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error fetching payment method");
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| ApiError::not_found(format!("Payment method {} not found", id)))?;

    Ok(Json(payment_method))
}

/// GET /api/ar/payment-methods - List payment methods (with optional filtering)
#[utoipa::path(get, path = "/api/ar/payment-methods", tag = "Payment Methods",
    params(ListPaymentMethodsQuery),
    responses(
        (status = 200, description = "Paginated payment methods", body = PaginatedResponse<PaymentMethod>),
    ),
    security(("bearer" = [])))]
pub async fn list_payment_methods(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListPaymentMethodsQuery>,
) -> Result<Json<PaginatedResponse<PaymentMethod>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    let total_items = payment_methods::count_payment_methods(
        &db,
        &app_id,
        query.customer_id,
        query.status.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Database error counting payment methods");
        ApiError::internal("Internal database error")
    })?;

    let payment_methods = payment_methods::list_payment_methods(
        &db,
        &app_id,
        query.customer_id,
        query.status.as_deref(),
        limit,
        offset,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Database error listing payment methods");
        ApiError::internal("Internal database error")
    })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(
        payment_methods,
        page,
        limit as i64,
        total_items,
    )))
}
