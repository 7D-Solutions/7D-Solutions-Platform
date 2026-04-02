use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{ApiError, Invoice, ListInvoicesQuery, PaginatedResponse};

/// GET /api/ar/invoices/:id - Get invoice by ID
#[utoipa::path(get, path = "/api/ar/invoices/{id}", tag = "Invoices",
    params(("id" = i32, Path, description = "Invoice ID")),
    responses(
        (status = 200, description = "Invoice found", body = Invoice),
        (status = 404, description = "Invoice not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_invoice(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Invoice>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let invoice = crate::domain::invoices::service::get_invoice(&db, &app_id, id).await?;

    Ok(Json(invoice))
}

/// GET /api/ar/invoices - List invoices (with optional filtering)
#[utoipa::path(get, path = "/api/ar/invoices", tag = "Invoices",
    params(ListInvoicesQuery),
    responses(
        (status = 200, description = "Paginated invoices", body = PaginatedResponse<Invoice>),
    ),
    security(("bearer" = [])))]
pub async fn list_invoices(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListInvoicesQuery>,
) -> Result<Json<PaginatedResponse<Invoice>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let result = crate::domain::invoices::service::list_invoices(&db, &app_id, query).await?;

    Ok(Json(result))
}
