use axum::{
    body::Body,
    extract::Multipart,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::domain::annotations::{render, types::Annotation};

/// POST /api/pdf/render-annotations
///
/// Accepts multipart form data with:
/// - `file`: PDF file bytes
/// - `annotations`: JSON array of Annotation objects
///
/// Returns the processed PDF bytes with annotations burned in.
pub async fn render_annotations(mut multipart: Multipart) -> Result<Response, Response> {
    let mut pdf_bytes: Option<Vec<u8>> = None;
    let mut annotations: Option<Vec<Annotation>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, &format!("Multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "file" => {
                let data = field.bytes().await.map_err(|e| {
                    error_response(StatusCode::BAD_REQUEST, &format!("Failed to read file: {e}"))
                })?;
                pdf_bytes = Some(data.to_vec());
            }
            "annotations" => {
                let data = field.bytes().await.map_err(|e| {
                    error_response(
                        StatusCode::BAD_REQUEST,
                        &format!("Failed to read annotations: {e}"),
                    )
                })?;
                let parsed: Vec<Annotation> = serde_json::from_slice(&data).map_err(|e| {
                    error_response(
                        StatusCode::BAD_REQUEST,
                        &format!("Invalid annotations JSON: {e}"),
                    )
                })?;
                annotations = Some(parsed);
            }
            _ => {
                // Skip unknown fields
            }
        }
    }

    let pdf_bytes = pdf_bytes
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Missing 'file' field"))?;
    let annotations = annotations
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Missing 'annotations' field"))?;

    if annotations.is_empty() {
        // No annotations to render — return original PDF
        return Ok(pdf_response(pdf_bytes));
    }

    let result = tokio::task::spawn_blocking(move || {
        render::render_annotations(&pdf_bytes, &annotations)
    })
    .await
    .map_err(|e| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Render task failed: {e}"),
        )
    })?;

    match result {
        Ok(output_bytes) => Ok(pdf_response(output_bytes)),
        Err(render::RenderError::TooLarge) => Err(error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            &render::RenderError::TooLarge.to_string(),
        )),
        Err(render::RenderError::InvalidMagic) => Err(error_response(
            StatusCode::BAD_REQUEST,
            &render::RenderError::InvalidMagic.to_string(),
        )),
        Err(render::RenderError::InvalidPage(pg, total)) => Err(error_response(
            StatusCode::BAD_REQUEST,
            &format!("Invalid page number {pg}: document has {total} pages"),
        )),
        Err(e) => Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Render error: {e}"),
        )),
    }
}

fn pdf_response(bytes: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/pdf")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"annotated.pdf\"",
        )
        .body(Body::from(bytes))
        .unwrap()
}

fn error_response(status: StatusCode, message: &str) -> Response {
    let code = match status {
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
        _ => "internal_error",
    };
    let body = serde_json::json!({ "error": code, "message": message });
    (status, Json(body)).into_response()
}
