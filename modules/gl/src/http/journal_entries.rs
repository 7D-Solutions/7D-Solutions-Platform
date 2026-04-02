//! POST /api/gl/journal-entries — create a journal entry via HTTP.
//!
//! Used by consolidation (and any module that needs to post GL entries
//! synchronously rather than through the event bus).

use axum::{extract::State, http::StatusCode, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::auth::with_request_id;
use crate::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use crate::services::journal_service;
use crate::AppState;

/// UUID v5 namespace for HTTP-originated journal entries.
const HTTP_JOURNAL_NS: Uuid = Uuid::from_bytes([
    0x7d, 0x53, 0x6f, 0x6c, 0x75, 0x74, 0x69, 0x6f,
    0x6e, 0x73, 0x47, 0x4c, 0x4a, 0x45, 0x48, 0x54,
]);

#[derive(Debug, Deserialize, ToSchema)]
pub struct PostJournalEntryRequest {
    /// Originating module (e.g. "consolidation-elimination")
    pub source_module: String,
    /// Accounting date (YYYY-MM-DD)
    pub posting_date: String,
    /// ISO 4217 currency code
    pub currency: String,
    /// Document type
    pub source_doc_type: SourceDocType,
    /// Unique ID of the source document (also used for idempotency)
    pub source_doc_id: String,
    /// Human-readable description
    pub description: String,
    /// At least 2 balanced lines
    pub lines: Vec<JournalLine>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PostJournalEntryResponse {
    pub journal_entry_id: Uuid,
}

#[utoipa::path(post, path = "/api/gl/journal-entries", tag = "Journal Entries",
    request_body = PostJournalEntryRequest,
    responses(
        (status = 201, description = "Journal entry created", body = PostJournalEntryResponse),
        (status = 400, description = "Validation error", body = platform_http_contracts::ApiError),
        (status = 409, description = "Duplicate (idempotent)", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn create_journal_entry(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<PostJournalEntryRequest>,
) -> Result<(StatusCode, Json<PostJournalEntryResponse>), ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    // Deterministic event ID from tenant + source_doc_id for idempotency.
    let event_id = Uuid::new_v5(
        &HTTP_JOURNAL_NS,
        format!("{}:{}", tenant_id, req.source_doc_id).as_bytes(),
    );

    let payload = GlPostingRequestV1 {
        posting_date: req.posting_date,
        currency: req.currency,
        source_doc_type: req.source_doc_type,
        source_doc_id: req.source_doc_id,
        description: req.description,
        lines: req.lines,
    };

    let entry_id = journal_service::process_gl_posting_request(
        &app_state.pool,
        event_id,
        &tenant_id,
        &req.source_module,
        "http.journal-entries",
        &payload,
        None,
    )
    .await
    .map_err(|e| {
        let api_err = match &e {
            journal_service::JournalError::DuplicateEvent(_) => {
                ApiError::conflict(e.to_string())
            }
            journal_service::JournalError::Validation(_) => {
                ApiError::bad_request(e.to_string())
            }
            journal_service::JournalError::InvalidDate(_) => {
                ApiError::bad_request(e.to_string())
            }
            journal_service::JournalError::Period(_) => {
                ApiError::bad_request(e.to_string())
            }
            _ => ApiError::internal("Journal entry creation failed"),
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok((
        StatusCode::CREATED,
        Json(PostJournalEntryResponse {
            journal_entry_id: entry_id,
        }),
    ))
}
