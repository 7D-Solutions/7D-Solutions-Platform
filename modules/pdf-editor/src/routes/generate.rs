//! PDF generation HTTP handler.
//!
//! POST /api/pdf/forms/submissions/:id/generate
//!
//! Accepts multipart form data with:
//! - `file`: PDF template bytes
//! - `tenant_id`: query parameter for tenant isolation
//!
//! Looks up the submission + template fields from DB, overlays field
//! values onto the PDF at pdf_position coordinates, emits
//! `pdf.form.generated` event, and returns the filled PDF.

use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::forms::FieldRepo;
use crate::domain::generate::generate_filled_pdf;
use crate::domain::submissions::SubmissionRepo;
use crate::event_bus::{create_pdf_editor_envelope, enqueue_event};

use super::templates::TenantQuery;

/// Event payload for pdf.form.generated.
#[derive(Debug, Clone, Serialize)]
struct FormGeneratedPayload {
    tenant_id: String,
    submission_id: Uuid,
    template_id: Uuid,
}

/// POST /api/pdf/forms/submissions/:id/generate
pub async fn generate_pdf(
    State(pool): State<PgPool>,
    Path(submission_id): Path<Uuid>,
    Query(q): Query<TenantQuery>,
    multipart: Multipart,
) -> Result<Response, Response> {
    if q.tenant_id.trim().is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "tenant_id is required",
        ));
    }

    let pdf_bytes = extract_pdf_bytes(multipart).await?;

    // Look up submission
    let submission = SubmissionRepo::find_by_id(&pool, submission_id, &q.tenant_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "DB error looking up submission");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        })?
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "Submission not found"))?;

    if submission.status != "submitted" {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Submission must be in 'submitted' status to generate PDF",
        ));
    }

    // Load template fields
    let fields = FieldRepo::list_by_template(&pool, submission.template_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "DB error loading fields");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        })?;

    let field_data = submission.field_data.clone();
    let result = tokio::task::spawn_blocking(move || {
        generate_filled_pdf(&pdf_bytes, &fields, &field_data)
    })
    .await
    .map_err(|e| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Generation task failed: {e}"),
        )
    })?;

    use crate::domain::generate::GenerateError;

    let output_bytes = match result {
        Ok(bytes) => bytes,
        Err(GenerateError::TooLarge) => {
            return Err(error_response(StatusCode::PAYLOAD_TOO_LARGE, "PDF too large"));
        }
        Err(GenerateError::InvalidMagic) => {
            return Err(error_response(StatusCode::BAD_REQUEST, "Invalid PDF file"));
        }
        Err(GenerateError::InvalidPage(pg, total)) => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                &format!("Field references page {pg} but PDF has {total} pages"),
            ));
        }
        Err(e) => {
            tracing::error!(error = %e, "PDF generation error");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Generation error: {e}"),
            ));
        }
    };

    // Emit pdf.form.generated event
    let payload = FormGeneratedPayload {
        tenant_id: submission.tenant_id.clone(),
        submission_id: submission.id,
        template_id: submission.template_id,
    };
    let envelope = create_pdf_editor_envelope(
        Uuid::new_v4(),
        submission.tenant_id.clone(),
        "pdf.form.generated".to_string(),
        None,
        None,
        "DATA_MUTATION".to_string(),
        payload,
    );

    let mut tx = pool.begin().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to start transaction");
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
    })?;

    enqueue_event(&mut tx, "pdf.form.generated", &envelope)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to enqueue event");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        })?;

    tx.commit().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to commit transaction");
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
    })?;

    Ok(pdf_response(output_bytes))
}

/// Extract PDF bytes from the multipart `file` field.
async fn extract_pdf_bytes(mut multipart: Multipart) -> Result<Vec<u8>, Response> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, &format!("Multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            let data = field.bytes().await.map_err(|e| {
                error_response(StatusCode::BAD_REQUEST, &format!("Failed to read file: {e}"))
            })?;
            return Ok(data.to_vec());
        }
    }
    Err(error_response(StatusCode::BAD_REQUEST, "Missing 'file' field"))
}

fn pdf_response(bytes: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/pdf")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"generated.pdf\"",
        )
        .body(Body::from(bytes))
        .unwrap()
}

fn error_response(status: StatusCode, message: &str) -> Response {
    let body = json!({ "error": message });
    (status, Json(body)).into_response()
}
