use axum::{
    extract::{Path, State},
    Extension, Json,
};
use event_bus::TracingContext;
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::domain::usage as usage_repo;
use crate::models::{ApiError, CaptureUsageRequest, UsageRecord};

// ============================================================================
// USAGE INGESTION (bd-23z)
// ============================================================================

#[utoipa::path(post, path = "/api/ar/usage", tag = "Usage",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Usage record captured", body = serde_json::Value),
        (status = 400, description = "Validation error", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
/// POST /api/ar/usage — Capture metered usage (idempotent)
///
/// Inserts a usage record into ar_metered_usage and emits ar.usage_captured
/// into the outbox atomically. Duplicate submissions with the same idempotency_key
/// are a no-op that returns the original record.
///
/// Guard → Mutation → Outbox atomicity: all three happen in one transaction.
pub async fn capture_usage(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CaptureUsageRequest>,
) -> Result<Json<UsageRecord>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;
    let tracing_ctx = tracing_ctx.map(|Extension(c)| c).unwrap_or_default();

    // Guard: check for duplicate idempotency_key (no-op return of original)
    let existing: Option<UsageRecord> =
        usage_repo::find_by_idempotency_key(&db, req.idempotency_key)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "DB error checking usage idempotency");
                ApiError::internal("Internal database error")
            })?;

    if let Some(record) = existing {
        tracing::info!(
            idempotency_key = %req.idempotency_key,
            "Usage capture is duplicate — returning original record (idempotent no-op)"
        );
        return Ok(Json(record));
    }

    // Resolve customer_id integer from string external_customer_id or numeric string
    let customer_id: i32 = req.customer_id.parse().map_err(|_| {
        ApiError::bad_request(format!(
            "customer_id must be a numeric AR customer id, got: {}",
            req.customer_id
        ))
    })?;

    let quantity_minor: i64 = req.unit_price_minor;

    // Begin transaction: insert usage + outbox event atomically
    let mut tx = db.begin().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to begin transaction");
        ApiError::internal("Internal database error")
    })?;

    // Mutation: insert usage record
    let record = usage_repo::insert_usage(
        &mut *tx,
        &app_id,
        customer_id,
        &req.metric_name,
        req.quantity,
        quantity_minor as i32,
        req.period_start.naive_utc(),
        req.period_end.naive_utc(),
        req.idempotency_key,
        &req.unit,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to insert usage record");
        ApiError::internal("Internal database error")
    })?;

    // Outbox: emit ar.usage_captured in the same transaction
    use crate::events::contracts::{
        build_usage_captured_envelope, UsageCapturedPayload, EVENT_TYPE_USAGE_CAPTURED,
    };

    let usage_payload = UsageCapturedPayload {
        usage_id: record.usage_uuid,
        tenant_id: app_id.clone(),
        customer_id: record.customer_id.to_string(),
        metric_name: record.metric_name.clone(),
        quantity: req.quantity,
        unit: record.unit.clone(),
        period_start: req.period_start,
        period_end: req.period_end,
        subscription_id: req.subscription_id.map(|_| record.usage_uuid), // placeholder
        captured_at: chrono::Utc::now(),
    };

    let envelope = build_usage_captured_envelope(
        req.idempotency_key, // event_id = idempotency_key for determinism
        app_id.clone(),
        req.idempotency_key.to_string(), // correlation_id
        None,
        usage_payload,
    )
    .with_tracing_context(&tracing_ctx);

    crate::events::outbox::enqueue_event_tx(
        &mut tx,
        EVENT_TYPE_USAGE_CAPTURED,
        "usage",
        &record.id.to_string(),
        &envelope,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to enqueue ar.usage_captured event");
        ApiError::internal("Internal database error")
    })?;

    // Commit: usage insert + outbox event commit atomically
    tx.commit().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to commit usage transaction");
        ApiError::internal("Internal database error")
    })?;

    tracing::info!(
        usage_id = %record.usage_uuid,
        metric_name = %record.metric_name,
        "Usage captured and outbox event enqueued"
    );

    Ok(Json(record))
}

// ============================================================================
// POST /api/ar/invoices/{id}/bill-usage  (bd-n9j)
// ============================================================================

#[derive(serde::Deserialize)]
pub struct BillUsageHttpRequest {
    pub customer_id: i32,
    pub period_start: chrono::DateTime<chrono::Utc>,
    pub period_end: chrono::DateTime<chrono::Utc>,
    pub correlation_id: String,
}

#[utoipa::path(post, path = "/api/ar/invoices/{id}/bill-usage", tag = "Usage",
    params(("id" = i32, Path, description = "Invoice ID")),
    request_body = serde_json::Value,
    responses((status = 200, description = "Usage billed to invoice", body = serde_json::Value)),
    security(("bearer" = [])))]
pub async fn bill_usage_route(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(invoice_id): Path<i32>,
    Json(req): Json<BillUsageHttpRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use crate::usage_billing::{bill_usage_for_invoice, BillUsageRequest};

    let app_id = super::tenant::extract_tenant(&claims)?;

    bill_usage_for_invoice(
        &db,
        BillUsageRequest {
            app_id: app_id.clone(),
            invoice_id,
            customer_id: req.customer_id,
            period_start: req.period_start,
            period_end: req.period_end,
            correlation_id: req.correlation_id,
        },
    )
    .await
    .map(|billing| {
        Json(serde_json::json!({
            "billed_count": billing.billed_count,
            "total_amount_minor": billing.total_amount_minor,
        }))
    })
    .map_err(|e| {
        tracing::error!(invoice_id = %invoice_id, error = %e, "bill-usage failed");
        ApiError::internal("Internal database error")
    })
}
