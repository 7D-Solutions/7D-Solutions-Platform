use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::credit_notes::{issue_credit_note, IssueCreditNoteRequest};
use crate::models::ErrorResponse;

// ============================================================================
// CREDIT NOTE HANDLER WRAPPER (bd-1gt)
// ============================================================================

/// POST /api/ar/invoices/{id}/credit-notes
///
/// Axum handler wrapper for the domain service `issue_credit_note`.
pub async fn issue_credit_note_route(
    State(db): State<PgPool>,
    Path(invoice_id): Path<i32>,
    Json(mut req): Json<IssueCreditNoteRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    use crate::credit_notes::IssueCreditNoteResult;
    req.invoice_id = invoice_id;
    match issue_credit_note(&db, req).await {
        Ok(IssueCreditNoteResult::Issued {
            credit_note_row_id,
            credit_note_id,
            issued_at,
        }) => Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({
                "status": "issued",
                "credit_note_row_id": credit_note_row_id,
                "credit_note_id": credit_note_id,
                "issued_at": issued_at,
            })),
        )),
        Ok(IssueCreditNoteResult::AlreadyProcessed {
            existing_row_id,
            credit_note_id,
        }) => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "already_processed",
                "existing_row_id": existing_row_id,
                "credit_note_id": credit_note_id,
            })),
        )),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("credit_note_error", format!("{:?}", e))),
        )),
    }
}

// ============================================================================
// POST /api/ar/invoices/{id}/credit-notes  (bd-1gt)
// ============================================================================

/// HTTP request body for issuing a credit note
#[derive(serde::Deserialize)]
pub struct IssueCreditNoteHttpRequest {
    /// Stable UUID for this credit note (idempotency anchor)
    credit_note_id: uuid::Uuid,
    /// Customer ID (string, as stored in AR schema)
    customer_id: String,
    /// Credit amount in minor currency units (e.g. cents), must be > 0
    amount_minor: i64,
    /// ISO 4217 currency code (lowercase, e.g. "usd")
    currency: String,
    /// Human-readable reason (e.g. "billing_error", "service_credit")
    reason: String,
    /// Optional reference to a usage record or line item
    reference_id: Option<String>,
    /// Who authorized this credit (optional)
    issued_by: Option<String>,
    /// Distributed trace correlation ID
    correlation_id: String,
    /// Causation ID linking this to the triggering event/action
    causation_id: Option<String>,
}

/// Response for a successfully issued credit note
#[derive(serde::Serialize)]
pub struct IssueCreditNoteResponse {
    credit_note_id: uuid::Uuid,
    credit_note_row_id: i32,
    invoice_id: i32,
    amount_minor: i64,
    currency: String,
    reason: String,
    issued_at: chrono::DateTime<chrono::Utc>,
    status: &'static str,
}

/// POST /api/ar/invoices/{id}/credit-notes
///
/// Issue a credit note against an invoice. Atomic: credit note row + outbox
/// event are committed together. Idempotent on `credit_note_id`.
pub async fn issue_credit_note_handler(
    State(pool): State<PgPool>,
    Path(invoice_id): Path<i32>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<IssueCreditNoteHttpRequest>,
) -> Result<(StatusCode, Json<IssueCreditNoteResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Extract tenant from verified JWT claims (C1 fix: was header-based)
    let app_id = super::tenant::extract_tenant(&claims)?;

    let req = IssueCreditNoteRequest {
        credit_note_id: body.credit_note_id,
        app_id,
        customer_id: body.customer_id,
        invoice_id,
        amount_minor: body.amount_minor,
        currency: body.currency.clone(),
        reason: body.reason.clone(),
        reference_id: body.reference_id,
        issued_by: body.issued_by,
        correlation_id: body.correlation_id,
        causation_id: body.causation_id,
    };

    match issue_credit_note(&pool, req).await {
        Ok(crate::credit_notes::IssueCreditNoteResult::Issued {
            credit_note_row_id,
            credit_note_id,
            issued_at,
        }) => {
            tracing::info!(
                credit_note_id = %credit_note_id,
                invoice_id = invoice_id,
                amount_minor = body.amount_minor,
                "Credit note issued"
            );
            Ok((
                StatusCode::CREATED,
                Json(IssueCreditNoteResponse {
                    credit_note_id,
                    credit_note_row_id,
                    invoice_id,
                    amount_minor: body.amount_minor,
                    currency: body.currency,
                    reason: body.reason,
                    issued_at,
                    status: "issued",
                }),
            ))
        }
        Ok(crate::credit_notes::IssueCreditNoteResult::AlreadyProcessed {
            existing_row_id,
            credit_note_id,
        }) => {
            tracing::info!(
                credit_note_id = %credit_note_id,
                existing_row_id = existing_row_id,
                "Credit note already issued (idempotent no-op)"
            );
            // Return 200 with the existing row to signal idempotent success
            Ok((
                StatusCode::OK,
                Json(IssueCreditNoteResponse {
                    credit_note_id,
                    credit_note_row_id: existing_row_id,
                    invoice_id,
                    amount_minor: body.amount_minor,
                    currency: body.currency,
                    reason: body.reason,
                    issued_at: chrono::Utc::now(), // approximate for already-issued
                    status: "issued",
                }),
            ))
        }
        Err(crate::credit_notes::CreditNoteError::InvoiceNotFound { invoice_id, app_id }) => {
            tracing::warn!(invoice_id = invoice_id, app_id = %app_id, "Credit note: invoice not found");
            Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse::new(
                    "invoice_not_found",
                    format!("Invoice {} not found", invoice_id),
                )),
            ))
        }
        Err(crate::credit_notes::CreditNoteError::InvalidAmount(n)) => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse::new(
                "invalid_amount",
                format!("amount_minor must be > 0, got {}", n),
            )),
        )),
        Err(crate::credit_notes::CreditNoteError::InvalidCurrency) => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse::new(
                "invalid_currency",
                "currency must not be empty",
            )),
        )),
        Err(crate::credit_notes::CreditNoteError::OverCreditBalance {
            invoice_id,
            invoice_amount_cents,
            existing_credits,
            requested,
        }) => {
            tracing::warn!(
                invoice_id,
                invoice_amount_cents,
                existing_credits,
                requested,
                "Credit note rejected: over-credit guard triggered"
            );
            Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorResponse::new(
                    "over_credit",
                    format!(
                        "Credit of {} exceeds remaining balance {} on invoice {}",
                        requested,
                        invoice_amount_cents - existing_credits,
                        invoice_id
                    ),
                )),
            ))
        }
        Err(crate::credit_notes::CreditNoteError::DatabaseError(msg)) => {
            tracing::error!("Credit note DB error: {}", msg);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    "Failed to issue credit note",
                )),
            ))
        }
    }
}
