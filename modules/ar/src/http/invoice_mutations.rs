use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{NaiveDate, Utc};
use event_bus::TracingContext;
use security::VerifiedClaims;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{ApiError, CreateInvoiceRequest, FinalizeInvoiceRequest, Invoice, UpdateInvoiceRequest};

// ============================================================================
// GL period pre-validation
// ============================================================================

/// Guard: check that the GL period containing `date` is open for `tenant_id`.
///
/// Returns `Err(422 PERIOD_CLOSED)` if the period is closed.
/// Returns `Ok(())` if open, if no period exists for the date (GL enforces on
/// the posting event), or if the GL pool is unreachable (fail-open).
async fn check_gl_period_open(
    gl_pool: &PgPool,
    tenant_id: &str,
    date: NaiveDate,
) -> Result<(), ApiError> {
    let result: sqlx::Result<Option<(Uuid, Option<chrono::DateTime<Utc>>)>> =
        sqlx::query_as(
            r#"
            SELECT id, closed_at
            FROM accounting_periods
            WHERE tenant_id = $1
              AND period_start <= $2
              AND period_end   >= $2
            LIMIT 1
            "#,
        )
        .bind(tenant_id)
        .bind(date)
        .fetch_optional(gl_pool)
        .await;

    match result {
        Err(e) => {
            tracing::warn!(tenant_id, %date, error = %e, "GL period check DB error — allowing (fail-open)");
            Ok(())
        }
        Ok(None) => Ok(()), // no period for date — GL will enforce on posting
        Ok(Some((_, None))) => Ok(()), // period exists and is open
        Ok(Some((_, Some(_)))) => Err(ApiError::new(
            422,
            "PERIOD_CLOSED",
            format!(
                "Period for {} is closed — request reopen or adjust the effective date",
                date
            ),
        )),
    }
}

/// POST /api/ar/invoices - Create a new invoice
#[utoipa::path(post, path = "/api/ar/invoices", tag = "Invoices",
    request_body = CreateInvoiceRequest,
    responses(
        (status = 201, description = "Invoice created", body = Invoice),
        (status = 400, description = "Validation error", body = platform_http_contracts::ApiError),
        (status = 422, description = "Period closed", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn create_invoice(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    party_client_ext: Option<Extension<std::sync::Arc<platform_client_party::PartiesClient>>>,
    gl_pool_ext: Option<Extension<std::sync::Arc<PgPool>>>,
    Json(req): Json<CreateInvoiceRequest>,
) -> Result<(StatusCode, Json<Invoice>), ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;
    let verified = claims.as_ref().map(|Extension(c)| c);
    let tracing_ctx = tracing_ctx.map(|Extension(c)| c).unwrap_or_default();
    let party_client = party_client_ext.as_ref().map(|Extension(c)| c.as_ref());

    // Period pre-validation: fail fast before any DB writes.
    // Effective date is billing_period_start if provided, otherwise today.
    if let Some(Extension(gl_pool)) = gl_pool_ext.as_ref() {
        let effective_date = req
            .billing_period_start
            .map(|dt| dt.date())
            .unwrap_or_else(|| Utc::now().naive_utc().date());
        check_gl_period_open(gl_pool.as_ref(), &app_id, effective_date).await?;
    }

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
