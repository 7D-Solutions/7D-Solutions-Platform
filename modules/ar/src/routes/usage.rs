use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use event_bus::TracingContext;
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{CaptureUsageRequest, ErrorResponse, UsageRecord};

// ============================================================================
// USAGE INGESTION (bd-23z)
// ============================================================================

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
) -> Result<Json<UsageRecord>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;
    let tracing_ctx = tracing_ctx.map(|Extension(c)| c).unwrap_or_default();

    // Guard: check for duplicate idempotency_key (no-op return of original)
    let existing: Option<UsageRecord> = sqlx::query_as::<_, UsageRecord>(
        r#"
        SELECT id, usage_uuid, idempotency_key, app_id, customer_id, metric_name,
               quantity, unit, unit_price_cents, period_start, period_end, recorded_at
        FROM ar_metered_usage
        WHERE idempotency_key = $1
        "#,
    )
    .bind(req.idempotency_key)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("DB error checking usage idempotency: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("database_error", format!("DB error: {}", e))),
        )
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
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                format!("customer_id must be a numeric AR customer id, got: {}", req.customer_id),
            )),
        )
    })?;

    let quantity_minor: i64 = req.unit_price_minor;

    // Begin transaction: insert usage + outbox event atomically
    let mut tx = db.begin().await.map_err(|e| {
        tracing::error!("Failed to begin transaction: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("database_error", format!("Failed to begin tx: {}", e))),
        )
    })?;

    // Mutation: insert usage record
    let record = sqlx::query_as::<_, UsageRecord>(
        r#"
        INSERT INTO ar_metered_usage (
            app_id, customer_id, metric_name, quantity, unit_price_cents,
            period_start, period_end, recorded_at,
            idempotency_key, usage_uuid, unit
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), $8, gen_random_uuid(), $9)
        RETURNING id, usage_uuid, idempotency_key, app_id, customer_id, metric_name,
                  quantity, unit, unit_price_cents, period_start, period_end, recorded_at
        "#,
    )
    .bind(&app_id)
    .bind(customer_id)
    .bind(&req.metric_name)
    .bind(req.quantity)
    .bind(quantity_minor as i32)
    .bind(req.period_start.naive_utc())
    .bind(req.period_end.naive_utc())
    .bind(req.idempotency_key)
    .bind(&req.unit)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Failed to insert usage record: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("database_error", format!("Failed to insert usage: {}", e))),
        )
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
        tracing::error!("Failed to enqueue ar.usage_captured event: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("outbox_error", format!("Failed to enqueue event: {}", e))),
        )
    })?;

    // Commit: usage insert + outbox event commit atomically
    tx.commit().await.map_err(|e| {
        tracing::error!("Failed to commit usage transaction: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("transaction_error", format!("Failed to commit: {}", e))),
        )
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

pub async fn bill_usage_route(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(invoice_id): Path<i32>,
    Json(req): Json<BillUsageHttpRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
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
    .map(|billing| Json(serde_json::json!({
        "billed_count": billing.billed_count,
        "total_amount_minor": billing.total_amount_minor,
    })))
    .map_err(|e| {
        tracing::error!(invoice_id = %invoice_id, error = %e, "bill-usage failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("database_error", e.to_string())),
        )
    })
}
