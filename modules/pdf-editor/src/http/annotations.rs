use axum::{
    body::Body,
    extract::Multipart,
    http::{header, StatusCode},
    response::Response,
};
use platform_http_contracts::ApiError;

use crate::domain::annotations::{render, types::Annotation};

/// POST /api/pdf/render-annotations
///
/// Accepts multipart form data with:
/// - `file`: PDF file bytes
/// - `annotations`: JSON array of Annotation objects
///
/// Returns the processed PDF bytes with annotations burned in.
/// This endpoint has a 50 MB body limit for PDF uploads.
#[utoipa::path(
    post, path = "/api/pdf/render-annotations", tag = "Annotations",
    responses(
        (status = 200, description = "Annotated PDF", content_type = "application/pdf"),
        (status = 400, body = ApiError), (status = 413, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn render_annotations(mut multipart: Multipart) -> Result<Response, ApiError> {
    let mut pdf_bytes: Option<Vec<u8>> = None;
    let mut annotations: Option<Vec<Annotation>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("Multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "file" => {
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::bad_request(format!("Failed to read file: {e}")))?;
                pdf_bytes = Some(data.to_vec());
            }
            "annotations" => {
                let data = field.bytes().await.map_err(|e| {
                    ApiError::bad_request(format!("Failed to read annotations: {e}"))
                })?;
                let parsed: Vec<Annotation> = serde_json::from_slice(&data)
                    .map_err(|e| ApiError::bad_request(format!("Invalid annotations JSON: {e}")))?;
                annotations = Some(parsed);
            }
            _ => {}
        }
    }

    let pdf_bytes = pdf_bytes.ok_or_else(|| ApiError::bad_request("Missing 'file' field"))?;
    let annotations =
        annotations.ok_or_else(|| ApiError::bad_request("Missing 'annotations' field"))?;

    if annotations.is_empty() {
        return Ok(pdf_response(pdf_bytes));
    }

    let result =
        tokio::task::spawn_blocking(move || render::render_annotations(&pdf_bytes, &annotations))
            .await
            .map_err(|e| ApiError::internal(format!("Render task failed: {e}")))?;

    match result {
        Ok(output_bytes) => Ok(pdf_response(output_bytes)),
        Err(render::RenderError::TooLarge) => Err(ApiError::new(
            413,
            "payload_too_large",
            render::RenderError::TooLarge.to_string(),
        )),
        Err(render::RenderError::InvalidMagic) => Err(ApiError::bad_request(
            render::RenderError::InvalidMagic.to_string(),
        )),
        Err(render::RenderError::InvalidPage(pg, total)) => Err(ApiError::bad_request(format!(
            "Invalid page number {pg}: document has {total} pages"
        ))),
        Err(e) => Err(ApiError::internal(format!("Render error: {e}"))),
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
        .expect("static PDF response headers are valid")
}
