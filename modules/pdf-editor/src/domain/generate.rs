//! PDF generation from form submissions.
//!
//! Overlays field values from a submitted form onto a PDF template at
//! the coordinates defined by each field's `pdf_position` JSONB.
//!
//! pdf_position format: { "x": f32, "y": f32, "page": u32, "font_size": f32 }
//! Coordinates use a top-left origin matching the frontend; we convert
//! to bottom-left (PDF native) during rendering.

use pdfium_render::prelude::*;
use serde::Deserialize;

use crate::domain::forms::FormField;

/// Maximum PDF file size (50 MB).
pub const MAX_PDF_SIZE: usize = 50 * 1024 * 1024;

const PDF_MAGIC: &[u8] = b"%PDF-";
const DEFAULT_FONT_SIZE: f32 = 12.0;

/// Parsed pdf_position from the JSONB column.
#[derive(Debug, Deserialize)]
struct PdfPosition {
    x: f32,
    y: f32,
    page: Option<u32>,
    #[serde(default)]
    font_size: Option<f32>,
}

#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    #[error("PDF exceeds maximum size of {MAX_PDF_SIZE} bytes")]
    TooLarge,
    #[error("Invalid PDF: does not start with %PDF- magic bytes")]
    InvalidMagic,
    #[error("PDFium error: {0}")]
    Pdfium(#[from] PdfiumError),
    #[error("Invalid page number {0}: document has {1} pages")]
    InvalidPage(u32, u32),
}

/// Validate that bytes look like a PDF.
pub fn validate_pdf(bytes: &[u8]) -> Result<(), GenerateError> {
    if bytes.len() > MAX_PDF_SIZE {
        return Err(GenerateError::TooLarge);
    }
    if bytes.len() < PDF_MAGIC.len() || &bytes[..PDF_MAGIC.len()] != PDF_MAGIC {
        return Err(GenerateError::InvalidMagic);
    }
    Ok(())
}

fn create_pdfium() -> Result<Pdfium, PdfiumError> {
    if let Ok(path) = std::env::var("PDFIUM_LIB_PATH") {
        let bindings = Pdfium::bind_to_library(path)?;
        return Ok(Pdfium::new(bindings));
    }
    Ok(Pdfium::default())
}

/// Overlay submitted field values onto a PDF template.
///
/// For each field that has a valid `pdf_position` and a corresponding
/// value in `field_data`, renders the value as text at the specified
/// coordinates. Checkbox fields render "Yes"/"No".
pub fn generate_filled_pdf(
    pdf_bytes: &[u8],
    fields: &[FormField],
    field_data: &serde_json::Value,
) -> Result<Vec<u8>, GenerateError> {
    validate_pdf(pdf_bytes)?;

    let pdfium = create_pdfium()?;
    let mut document = pdfium.load_pdf_from_byte_slice(pdf_bytes, None)?;
    let page_count = document.pages().len() as u32;

    // Collect rendering instructions first, group by page.
    let mut page_fields: std::collections::HashMap<u32, Vec<(PdfPosition, String)>> =
        std::collections::HashMap::new();

    for field in fields {
        let pos: PdfPosition = match serde_json::from_value(field.pdf_position.clone()) {
            Ok(p) => p,
            Err(_) => continue,
        };

        let value = match field_data.get(&field.field_key) {
            Some(v) => v,
            None => continue,
        };

        let text = format_field_value(&field.field_type, value);
        if text.is_empty() {
            continue;
        }

        let page_num = pos.page.unwrap_or(1);
        if page_num < 1 || page_num > page_count {
            return Err(GenerateError::InvalidPage(page_num, page_count));
        }

        page_fields.entry(page_num).or_default().push((pos, text));
    }

    // Get font token before borrowing pages (PdfFontToken is Copy, no lifetime).
    let font_token = document.fonts_mut().helvetica();

    for (page_num, entries) in &page_fields {
        let page_index = (*page_num - 1) as u16;
        let mut page = document.pages_mut().get(page_index)?;
        let page_height = page.height().value;

        for (pos, text) in entries {
            let font_size = pos.font_size.unwrap_or(DEFAULT_FONT_SIZE);

            // Convert top-left origin → bottom-left origin
            let pdf_y = page_height - pos.y - font_size;

            // create_text_object uses internal raw handles, avoiding borrow conflicts.
            let mut obj = page.objects_mut().create_text_object(
                PdfPoints::new(pos.x),
                PdfPoints::new(pdf_y),
                text,
                font_token,
                PdfPoints::new(font_size),
            )?;

            obj.set_fill_color(PdfColor::new(0, 0, 0, 255))?;
        }
    }

    Ok(document.save_to_bytes()?)
}

/// Format a field value as display text based on field type.
fn format_field_value(field_type: &str, value: &serde_json::Value) -> String {
    match field_type {
        "checkbox" => {
            if value.as_bool().unwrap_or(false) {
                "Yes".to_string()
            } else {
                "No".to_string()
            }
        }
        "number" => {
            if let Some(n) = value.as_f64() {
                if n.fract() == 0.0 {
                    format!("{}", n as i64)
                } else {
                    format!("{}", n)
                }
            } else if let Some(s) = value.as_str() {
                s.to_string()
            } else {
                String::new()
            }
        }
        _ => {
            if let Some(s) = value.as_str() {
                s.to_string()
            } else {
                value.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_pdf_magic() {
        assert!(validate_pdf(b"%PDF-1.7 test").is_ok());
        assert!(matches!(
            validate_pdf(b"not a pdf"),
            Err(GenerateError::InvalidMagic)
        ));
    }

    #[test]
    fn test_validate_pdf_too_large() {
        let mut data = vec![b'%', b'P', b'D', b'F', b'-'];
        data.resize(MAX_PDF_SIZE + 1, 0);
        assert!(matches!(validate_pdf(&data), Err(GenerateError::TooLarge)));
    }

    #[test]
    fn test_format_checkbox_true() {
        let v = serde_json::json!(true);
        assert_eq!(format_field_value("checkbox", &v), "Yes");
    }

    #[test]
    fn test_format_checkbox_false() {
        let v = serde_json::json!(false);
        assert_eq!(format_field_value("checkbox", &v), "No");
    }

    #[test]
    fn test_format_number_integer() {
        let v = serde_json::json!(42000);
        assert_eq!(format_field_value("number", &v), "42000");
    }

    #[test]
    fn test_format_number_float() {
        let v = serde_json::json!(3.14);
        assert_eq!(format_field_value("number", &v), "3.14");
    }

    #[test]
    fn test_format_text() {
        let v = serde_json::json!("Acme Corp");
        assert_eq!(format_field_value("text", &v), "Acme Corp");
    }

    #[test]
    fn test_format_date() {
        let v = serde_json::json!("2026-02-24");
        assert_eq!(format_field_value("date", &v), "2026-02-24");
    }

    #[test]
    fn test_format_dropdown() {
        let v = serde_json::json!("truck");
        assert_eq!(format_field_value("dropdown", &v), "truck");
    }
}
