//! PDF generation HTTP handler.
//!
//! POST /api/pdf/forms/submissions/:id/generate
//!
//! Accepts multipart form data with:
//! - `file`: PDF template bytes
//!
//! Looks up the submission + template fields from DB, overlays field
//! values onto the PDF at pdf_position coordinates, emits
//! `pdf.form.generated` event, and returns the filled PDF.

use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    http::{header, StatusCode},
    response::Response,
    Extension,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::forms::FieldRepo;
use crate::domain::generate::generate_filled_pdf;
use crate::domain::submissions::SubmissionRepo;
use crate::event_bus::{create_pdf_editor_envelope, enqueue_event};

use super::tenant::{extract_tenant, with_request_id};

/// Event payload for pdf.form.generated.
#[derive(Debug, Clone, Serialize)]
struct FormGeneratedPayload {
    tenant_id: String,
    submission_id: Uuid,
    template_id: Uuid,
}

/// POST /api/pdf/forms/submissions/:id/generate
#[utoipa::path(
    post, path = "/api/pdf/forms/submissions/{id}/generate", tag = "Generate",
    params(("id" = Uuid, Path, description = "Submission ID")),
    responses(
        (status = 200, description = "Generated PDF", content_type = "application/pdf"),
        (status = 400, body = ApiError), (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn generate_pdf(
    State(pool): State<PgPool>,
    Path(submission_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    multipart: Multipart,
) -> Result<Response, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    let pdf_bytes = extract_pdf_bytes(multipart)
        .await
        .map_err(|e| with_request_id(e, &ctx))?;

    // Look up submission
    let submission = SubmissionRepo::find_by_id(&pool, submission_id, &tenant_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "DB error looking up submission");
            with_request_id(ApiError::internal("Database error"), &ctx)
        })?
        .ok_or_else(|| with_request_id(ApiError::not_found("Submission not found"), &ctx))?;

    if submission.status != "submitted" {
        return Err(with_request_id(
            ApiError::bad_request("Submission must be in 'submitted' status to generate PDF"),
            &ctx,
        ));
    }

    // Load template fields
    let fields = FieldRepo::list_by_template(&pool, submission.template_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "DB error loading fields");
            with_request_id(ApiError::internal("Database error"), &ctx)
        })?;

    let field_data = submission.field_data.clone();
    let result =
        tokio::task::spawn_blocking(move || generate_filled_pdf(&pdf_bytes, &fields, &field_data))
            .await
            .map_err(|e| {
                with_request_id(
                    ApiError::internal(format!("Generation task failed: {e}")),
                    &ctx,
                )
            })?;

    use crate::domain::generate::GenerateError;

    let output_bytes = match result {
        Ok(bytes) => bytes,
        Err(GenerateError::TooLarge) => {
            return Err(with_request_id(
                ApiError::new(413, "payload_too_large", "PDF too large"),
                &ctx,
            ));
        }
        Err(GenerateError::InvalidMagic) => {
            return Err(with_request_id(
                ApiError::bad_request("Invalid PDF file"),
                &ctx,
            ));
        }
        Err(GenerateError::InvalidPage(pg, total)) => {
            return Err(with_request_id(
                ApiError::bad_request(format!(
                    "Field references page {pg} but PDF has {total} pages"
                )),
                &ctx,
            ));
        }
        Err(e) => {
            tracing::error!(error = %e, "PDF generation error");
            return Err(with_request_id(
                ApiError::internal(format!("Generation error: {e}")),
                &ctx,
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
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    enqueue_event(&mut tx, "pdf.form.generated", &envelope)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to enqueue event");
            with_request_id(ApiError::internal("Database error"), &ctx)
        })?;

    tx.commit().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to commit transaction");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    Ok(pdf_response(output_bytes))
}

/// Extract PDF bytes from the multipart `file` field.
async fn extract_pdf_bytes(mut multipart: Multipart) -> Result<Vec<u8>, ApiError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("Multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::bad_request(format!("Failed to read file: {e}")))?;
            return Ok(data.to_vec());
        }
    }
    Err(ApiError::bad_request("Missing 'file' field"))
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
        .expect("static PDF response headers are valid")
}
