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

/// HTTP request body for POST /api/ar/invoices/{id}/credit-notes.
/// The `invoice_id` is taken from the path, not the request body.
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct IssueCreditNoteBody {
    pub credit_note_id: uuid::Uuid,
    pub app_id: String,
    pub customer_id: String,
    pub amount_minor: i64,
    pub currency: String,
    pub reason: String,
    pub reference_id: Option<String>,
    pub issued_by: Option<String>,
    pub correlation_id: String,
    pub causation_id: Option<String>,
}

/// Response for a successfully processed credit note.
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct CreditNoteRouteResponse {
    pub status: &'static str,
    pub credit_note_id: uuid::Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_note_row_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_row_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[utoipa::path(post, path = "/api/ar/invoices/{id}/credit-notes", tag = "Credit Notes",
    params(("id" = i32, Path, description = "Invoice ID")),
    request_body = IssueCreditNoteBody,
    responses(
        (status = 201, description = "Credit note issued", body = CreditNoteRouteResponse),
        (status = 200, description = "Already processed (idempotent)", body = CreditNoteRouteResponse),
        (status = 400, description = "Validation error", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
/// POST /api/ar/invoices/{id}/credit-notes
///
/// Axum handler wrapper for the domain service `issue_credit_note`.
pub async fn issue_credit_note_route(
    State(db): State<PgPool>,
    Path(invoice_id): Path<i32>,
    Json(body): Json<IssueCreditNoteBody>,
) -> Result<(StatusCode, Json<CreditNoteRouteResponse>), ApiError> {
    use crate::credit_notes::IssueCreditNoteResult;
    let req = IssueCreditNoteRequest {
        credit_note_id: body.credit_note_id,
        app_id: body.app_id,
        customer_id: body.customer_id,
        invoice_id,
        amount_minor: body.amount_minor,
        currency: body.currency,
        reason: body.reason,
        reference_id: body.reference_id,
        issued_by: body.issued_by,
        correlation_id: body.correlation_id,
        causation_id: body.causation_id,
    };
    match issue_credit_note(&db, req).await {
        Ok(IssueCreditNoteResult::Issued {
            credit_note_row_id,
            credit_note_id,
            issued_at,
        }) => Ok((
            StatusCode::CREATED,
            Json(CreditNoteRouteResponse {
                status: "issued",
                credit_note_id,
                credit_note_row_id: Some(credit_note_row_id),
                existing_row_id: None,
                issued_at: Some(issued_at),
            }),
        )),
        Ok(IssueCreditNoteResult::AlreadyProcessed {
            existing_row_id,
            credit_note_id,
        }) => Ok((
            StatusCode::OK,
            Json(CreditNoteRouteResponse {
                status: "already_processed",
                credit_note_id,
                credit_note_row_id: None,
                existing_row_id: Some(existing_row_id),
                issued_at: None,
            }),
        )),
        Err(e) => {
            tracing::error!(error = %e, "Credit note error");
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
            Err(ApiError::not_found(format!(
                "Invoice {} not found",
                invoice_id
            )))
        }
        Err(crate::credit_notes::CreditNoteError::InvalidAmount(n)) => Err(ApiError::new(
            422,
            "validation_error",
            format!("amount_minor must be > 0, got {}", n),
        )),
        Err(crate::credit_notes::CreditNoteError::InvalidCurrency) => Err(ApiError::new(
            422,
            "validation_error",
            "currency must not be empty",
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
            Err(ApiError::new(
                422,
                "validation_error",
                format!(
                    "Credit of {} exceeds remaining balance {} on invoice {}",
                    requested,
                    invoice_amount_cents - existing_credits,
                    invoice_id
                ),
            ))
        }
        Err(crate::credit_notes::CreditNoteError::DatabaseError(msg)) => {
            tracing::error!(message = %msg, "Credit note DB error");
            Err(ApiError::internal("Internal database error"))
        }
        Err(crate::credit_notes::CreditNoteError::InvalidStatusTransition {
            expected,
            actual,
            ..
        }) => Err(ApiError::conflict(format!(
            "Expected status '{}', got '{}'",
            expected, actual
        ))),
        Err(crate::credit_notes::CreditNoteError::CreditMemoNotFound {
            credit_note_id, ..
        }) => Err(ApiError::not_found(format!(
            "Credit memo {} not found",
            credit_note_id
        ))),
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

#[utoipa::path(post, path = "/api/ar/credit-memos", tag = "Credit Notes",
    request_body = serde_json::Value,
    responses(
        (status = 201, description = "Credit memo created (draft)", body = serde_json::Value),
        (status = 200, description = "Already processed (idempotent)", body = serde_json::Value),
        (status = 404, description = "Invoice not found", body = platform_http_contracts::ApiError),
        (status = 422, description = "Validation error", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
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
        Err(crate::credit_notes::CreditNoteError::InvoiceNotFound { invoice_id, .. }) => Err(
            ApiError::not_found(format!("Invoice {} not found", invoice_id)),
        ),
        Err(crate::credit_notes::CreditNoteError::InvalidAmount(n)) => Err(ApiError::new(
            422,
            "validation_error",
            format!("amount_minor must be > 0, got {}", n),
        )),
        Err(crate::credit_notes::CreditNoteError::InvalidCurrency) => Err(ApiError::new(
            422,
            "validation_error",
            "currency must not be empty",
        )),
        Err(crate::credit_notes::CreditNoteError::OverCreditBalance {
            invoice_id,
            invoice_amount_cents,
            existing_credits,
            requested,
        }) => Err(ApiError::new(
            422,
            "validation_error",
            format!(
                "Credit of {} exceeds remaining balance {} on invoice {}",
                requested,
                invoice_amount_cents - existing_credits,
                invoice_id
            ),
        )),
        Err(crate::credit_notes::CreditNoteError::DatabaseError(msg)) => {
            tracing::error!(message = %msg, "Credit memo create DB error");
            Err(ApiError::internal("Internal database error"))
        }
        Err(e) => {
            tracing::error!(error = %e, "Credit memo create error");
            Err(ApiError::internal("Internal database error"))
        }
    }
}

#[utoipa::path(post, path = "/api/ar/credit-memos/{id}/approve", tag = "Credit Notes",
    params(("id" = uuid::Uuid, Path, description = "Credit memo ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Credit memo approved", body = serde_json::Value),
        (status = 404, description = "Credit memo not found", body = platform_http_contracts::ApiError),
        (status = 409, description = "Invalid status transition", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
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
        }) => Err(ApiError::not_found(format!(
            "Credit memo {} not found",
            credit_note_id
        ))),
        Err(crate::credit_notes::CreditNoteError::InvalidStatusTransition {
            expected,
            actual,
            ..
        }) => Err(ApiError::conflict(format!(
            "Expected status '{}', got '{}'",
            expected, actual
        ))),
        Err(crate::credit_notes::CreditNoteError::DatabaseError(msg)) => {
            tracing::error!(message = %msg, "Credit memo approve DB error");
            Err(ApiError::internal("Internal database error"))
        }
        Err(e) => {
            tracing::error!(error = %e, "Credit memo approve error");
            Err(ApiError::internal("Internal database error"))
        }
    }
}

#[utoipa::path(post, path = "/api/ar/credit-memos/{id}/issue", tag = "Credit Notes",
    params(("id" = uuid::Uuid, Path, description = "Credit memo ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Credit memo issued", body = serde_json::Value),
        (status = 404, description = "Credit memo not found", body = platform_http_contracts::ApiError),
        (status = 409, description = "Invalid status transition", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
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
        }) => Err(ApiError::not_found(format!(
            "Credit memo {} not found",
            credit_note_id
        ))),
        Err(crate::credit_notes::CreditNoteError::InvalidStatusTransition {
            expected,
            actual,
            ..
        }) => Err(ApiError::conflict(format!(
            "Expected status '{}', got '{}'",
            expected, actual
        ))),
        Err(crate::credit_notes::CreditNoteError::DatabaseError(msg)) => {
            tracing::error!(message = %msg, "Credit memo issue DB error");
            Err(ApiError::internal("Internal database error"))
        }
        Err(e) => {
            tracing::error!(error = %e, "Credit memo issue error");
            Err(ApiError::internal("Internal database error"))
        }
    }
}
