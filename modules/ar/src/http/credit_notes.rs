use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::credit_notes::{
    approve_credit_memo, create_credit_memo, issue_credit_memo, issue_credit_note,
    ApproveCreditMemoRequest, CreateCreditMemoRequest, IssueCreditMemoRequest,
    IssueCreditNoteRequest,
};
use crate::models::ApiError;

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
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
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
        Err(e) => {
            tracing::error!("Credit note error: {:?}", e);
            let (status, msg) = match &e {
                crate::credit_notes::CreditNoteError::DatabaseError(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal database error".to_string(),
                ),
                crate::credit_notes::CreditNoteError::InvalidAmount(n) => (
                    StatusCode::BAD_REQUEST,
                    format!("Amount must be > 0, got {}", n),
                ),
                other => (StatusCode::BAD_REQUEST, format!("{}", other)),
            };
            Err(ApiError::new(status.as_u16(), "credit_note_error", msg))
        }
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
) -> Result<(StatusCode, Json<IssueCreditNoteResponse>), ApiError> {
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
            Err(ApiError::not_found(format!("Invoice {} not found", invoice_id)))
        }
        Err(crate::credit_notes::CreditNoteError::InvalidAmount(n)) => Err(ApiError::new(422, "validation_error", format!("amount_minor must be > 0, got {}", n))),
        Err(crate::credit_notes::CreditNoteError::InvalidCurrency) => Err(ApiError::new(422, "validation_error", "currency must not be empty")),
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
            Err(ApiError::new(422, "validation_error", format!(
                        "Credit of {} exceeds remaining balance {} on invoice {}",
                        requested,
                        invoice_amount_cents - existing_credits,
                        invoice_id
                    )))
        }
        Err(crate::credit_notes::CreditNoteError::DatabaseError(msg)) => {
            tracing::error!("Credit note DB error: {}", msg);
            Err(ApiError::internal("Internal database error"))
        }
        Err(crate::credit_notes::CreditNoteError::InvalidStatusTransition {
            expected,
            actual,
            ..
        }) => Err(ApiError::conflict(format!("Expected status '{}', got '{}'", expected, actual))),
        Err(crate::credit_notes::CreditNoteError::CreditMemoNotFound {
            credit_note_id, ..
        }) => Err(ApiError::not_found(format!("Credit memo {} not found", credit_note_id))),
    }
}

#[derive(serde::Deserialize)]
pub struct CreateCreditMemoHttpRequest {
    pub credit_note_id: uuid::Uuid,
    pub customer_id: String,
    pub invoice_id: i32,
    pub amount_minor: i64,
    pub currency: String,
    pub reason: String,
    pub reference_id: Option<String>,
    pub created_by: Option<String>,
    pub create_idempotency_key: uuid::Uuid,
    pub correlation_id: String,
    pub causation_id: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct ApproveCreditMemoHttpRequest {
    pub approved_by: Option<String>,
    pub correlation_id: String,
    pub causation_id: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct IssueCreditMemoHttpRequest {
    pub issued_by: Option<String>,
    pub issue_idempotency_key: uuid::Uuid,
    pub correlation_id: String,
    pub causation_id: Option<String>,
}

pub async fn create_credit_memo_handler(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<CreateCreditMemoHttpRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;
    match create_credit_memo(
        &pool,
        CreateCreditMemoRequest {
            credit_note_id: body.credit_note_id,
            app_id,
            customer_id: body.customer_id,
            invoice_id: body.invoice_id,
            amount_minor: body.amount_minor,
            currency: body.currency,
            reason: body.reason,
            reference_id: body.reference_id,
            created_by: body.created_by,
            create_idempotency_key: body.create_idempotency_key,
            correlation_id: body.correlation_id,
            causation_id: body.causation_id,
        },
    )
    .await
    {
        Ok(crate::credit_notes::CreateCreditMemoResult::Created {
            credit_note_row_id,
            credit_note_id,
            created_at,
        }) => Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({
                "status": "draft",
                "credit_note_row_id": credit_note_row_id,
                "credit_note_id": credit_note_id,
                "created_at": created_at,
            })),
        )),
        Ok(crate::credit_notes::CreateCreditMemoResult::AlreadyProcessed {
            existing_row_id,
            credit_note_id,
        }) => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "already_processed",
                "credit_note_row_id": existing_row_id,
                "credit_note_id": credit_note_id,
            })),
        )),
        Err(crate::credit_notes::CreditNoteError::InvoiceNotFound { invoice_id, .. }) => Err(ApiError::not_found(format!("Invoice {} not found", invoice_id))),
        Err(crate::credit_notes::CreditNoteError::InvalidAmount(n)) => Err(ApiError::new(422, "validation_error", format!("amount_minor must be > 0, got {}", n))),
        Err(crate::credit_notes::CreditNoteError::InvalidCurrency) => Err(ApiError::new(422, "validation_error", "currency must not be empty")),
        Err(crate::credit_notes::CreditNoteError::OverCreditBalance {
            invoice_id,
            invoice_amount_cents,
            existing_credits,
            requested,
        }) => Err(ApiError::new(422, "validation_error", format!(
                    "Credit of {} exceeds remaining balance {} on invoice {}",
                    requested,
                    invoice_amount_cents - existing_credits,
                    invoice_id
                ))),
        Err(crate::credit_notes::CreditNoteError::DatabaseError(msg)) => {
            tracing::error!("Credit memo create DB error: {}", msg);
            Err(ApiError::internal("Internal database error"))
        }
        Err(e) => {
            tracing::error!("Credit memo create error: {:?}", e);
            Err(ApiError::internal("Internal database error"))
        }
    }
}

pub async fn approve_credit_memo_handler(
    State(pool): State<PgPool>,
    Path(credit_note_id): Path<uuid::Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<ApproveCreditMemoHttpRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;
    match approve_credit_memo(
        &pool,
        ApproveCreditMemoRequest {
            app_id,
            credit_note_id,
            approved_by: body.approved_by,
            correlation_id: body.correlation_id,
            causation_id: body.causation_id,
        },
    )
    .await
    {
        Ok(crate::credit_notes::ApproveCreditMemoResult::Approved {
            credit_note_row_id,
            approved_at,
            ..
        }) => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "approved",
                "credit_note_row_id": credit_note_row_id,
                "approved_at": approved_at,
            })),
        )),
        Ok(crate::credit_notes::ApproveCreditMemoResult::AlreadyApproved {
            credit_note_row_id,
            ..
        }) => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "approved",
                "credit_note_row_id": credit_note_row_id,
                "already_approved": true,
            })),
        )),
        Err(crate::credit_notes::CreditNoteError::CreditMemoNotFound {
            credit_note_id, ..
        }) => Err(ApiError::not_found(format!("Credit memo {} not found", credit_note_id))),
        Err(crate::credit_notes::CreditNoteError::InvalidStatusTransition {
            expected,
            actual,
            ..
        }) => Err(ApiError::conflict(format!("Expected status '{}', got '{}'", expected, actual))),
        Err(crate::credit_notes::CreditNoteError::DatabaseError(msg)) => {
            tracing::error!("Credit memo approve DB error: {}", msg);
            Err(ApiError::internal("Internal database error"))
        }
        Err(e) => {
            tracing::error!("Credit memo approve error: {:?}", e);
            Err(ApiError::internal("Internal database error"))
        }
    }
}

pub async fn issue_credit_memo_handler(
    State(pool): State<PgPool>,
    Path(credit_note_id): Path<uuid::Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<IssueCreditMemoHttpRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;
    match issue_credit_memo(
        &pool,
        IssueCreditMemoRequest {
            app_id,
            credit_note_id,
            issued_by: body.issued_by,
            issue_idempotency_key: body.issue_idempotency_key,
            correlation_id: body.correlation_id,
            causation_id: body.causation_id,
        },
    )
    .await
    {
        Ok(crate::credit_notes::IssueCreditMemoResult::Issued {
            credit_note_row_id,
            issued_at,
            ..
        }) => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "issued",
                "credit_note_row_id": credit_note_row_id,
                "issued_at": issued_at,
            })),
        )),
        Ok(crate::credit_notes::IssueCreditMemoResult::AlreadyProcessed {
            existing_row_id,
            ..
        }) => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "issued",
                "credit_note_row_id": existing_row_id,
                "already_processed": true,
            })),
        )),
        Err(crate::credit_notes::CreditNoteError::CreditMemoNotFound {
            credit_note_id, ..
        }) => Err(ApiError::not_found(format!("Credit memo {} not found", credit_note_id))),
        Err(crate::credit_notes::CreditNoteError::InvalidStatusTransition {
            expected,
            actual,
            ..
        }) => Err(ApiError::conflict(format!("Expected status '{}', got '{}'", expected, actual))),
        Err(crate::credit_notes::CreditNoteError::DatabaseError(msg)) => {
            tracing::error!("Credit memo issue DB error: {}", msg);
            Err(ApiError::internal("Internal database error"))
        }
        Err(e) => {
            tracing::error!("Credit memo issue error: {:?}", e);
            Err(ApiError::internal("Internal database error"))
        }
    }
}
