use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use event_bus::TracingContext;
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{ApiError, CreateInvoiceRequest, FinalizeInvoiceRequest, Invoice, UpdateInvoiceRequest};

/// POST /api/ar/invoices - Create a new invoice
#[utoipa::path(post, path = "/api/ar/invoices", tag = "Invoices",
    request_body = CreateInvoiceRequest,
    responses(
        (status = 201, description = "Invoice created", body = Invoice),
        (status = 400, description = "Validation error", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn create_invoice(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    party_client_ext: Option<Extension<std::sync::Arc<platform_client_party::PartiesClient>>>,
    Json(req): Json<CreateInvoiceRequest>,
) -> Result<(StatusCode, Json<Invoice>), ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;
    let verified = claims.as_ref().map(|Extension(c)| c);
    let tracing_ctx = tracing_ctx.map(|Extension(c)| c).unwrap_or_default();
    let party_client = party_client_ext.as_ref().map(|Extension(c)| c.as_ref());

    let invoice =
        crate::domain::invoices::service::create_invoice(&db, &app_id, verified, tracing_ctx, req, party_client)
            .await?;

    Ok((StatusCode::CREATED, Json(invoice)))
}

/// PUT /api/ar/invoices/:id - Update invoice
#[utoipa::path(put, path = "/api/ar/invoices/{id}", tag = "Invoices",
    params(("id" = i32, Path, description = "Invoice ID")),
    request_body = UpdateInvoiceRequest,
    responses(
        (status = 200, description = "Invoice updated", body = Invoice),
        (status = 404, description = "Invoice not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn update_invoice(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<UpdateInvoiceRequest>,
) -> Result<Json<Invoice>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let invoice =
        crate::domain::invoices::service::update_invoice(&db, &app_id, id, req).await?;

    Ok(Json(invoice))
}

/// POST /api/ar/invoices/:id/finalize - Mark invoice as finalized (open or paid)
#[utoipa::path(post, path = "/api/ar/invoices/{id}/finalize", tag = "Invoices",
    params(("id" = i32, Path, description = "Invoice ID")),
    request_body = FinalizeInvoiceRequest,
    responses(
        (status = 200, description = "Invoice finalized", body = Invoice),
        (status = 400, description = "Invalid status for finalization", body = platform_http_contracts::ApiError),
        (status = 404, description = "Invoice not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn finalize_invoice(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<i32>,
    Json(req): Json<FinalizeInvoiceRequest>,
) -> Result<Json<Invoice>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;
    let tracing_ctx = tracing_ctx.map(|Extension(c)| c).unwrap_or_default();

    let invoice =
        crate::domain::invoices::service::finalize_invoice(&db, &app_id, tracing_ctx, id, req)
            .await?;

    Ok(Json(invoice))
}
