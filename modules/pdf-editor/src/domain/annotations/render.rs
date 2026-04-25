//! Annotation rendering onto PDF pages using pdfium-render.
//!
//! All rendering uses page-level object creation APIs (create_text_object,
//! create_image_object, etc.) to avoid borrow conflicts between
//! PdfDocument and PdfPage lifetimes.

use pdfium_render::prelude::*;

use super::renderers;
use super::types::{Annotation, AnnotationType};

/// Maximum PDF file size (50 MB).
pub const MAX_PDF_SIZE: usize = 50 * 1024 * 1024;

const PDF_MAGIC: &[u8] = b"%PDF-";

fn create_pdfium() -> Result<Pdfium, PdfiumError> {
    if let Ok(path) = std::env::var("PDFIUM_LIB_PATH") {
        let bindings = Pdfium::bind_to_library(path)?;
        return Ok(Pdfium::new(bindings));
    }
    Ok(Pdfium::default())
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("PDF exceeds maximum size of {MAX_PDF_SIZE} bytes")]
    TooLarge,
    #[error("Invalid PDF: does not start with %PDF- magic bytes")]
    InvalidMagic,
    #[error("PDFium error: {0}")]
    Pdfium(#[from] PdfiumError),
    #[error("Invalid page number {0}: document has {1} pages")]
    InvalidPage(u32, u32),
    #[error("Invalid color value: {0}")]
    InvalidColor(String),
    #[error("Base64 decode error: {0}")]
    Base64(String),
    #[error("Image decode error: {0}")]
    ImageDecode(String),
}

pub fn validate_pdf(bytes: &[u8]) -> Result<(), RenderError> {
    if bytes.len() > MAX_PDF_SIZE {
        return Err(RenderError::TooLarge);
    }
    if bytes.len() < PDF_MAGIC.len() || &bytes[..PDF_MAGIC.len()] != PDF_MAGIC {
        return Err(RenderError::InvalidMagic);
    }
    Ok(())
}

pub(crate) fn parse_hex_color(s: &str) -> Result<(u8, u8, u8), RenderError> {
    let s = s.trim_start_matches('#');
    match s.len() {
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16)
                .map_err(|_| RenderError::InvalidColor(s.to_string()))?;
            let g = u8::from_str_radix(&s[2..4], 16)
                .map_err(|_| RenderError::InvalidColor(s.to_string()))?;
            let b = u8::from_str_radix(&s[4..6], 16)
                .map_err(|_| RenderError::InvalidColor(s.to_string()))?;
            Ok((r, g, b))
        }
        3 => {
            let r = u8::from_str_radix(&s[0..1], 16)
                .map_err(|_| RenderError::InvalidColor(s.to_string()))?;
            let g = u8::from_str_radix(&s[1..2], 16)
                .map_err(|_| RenderError::InvalidColor(s.to_string()))?;
            let b = u8::from_str_radix(&s[2..3], 16)
                .map_err(|_| RenderError::InvalidColor(s.to_string()))?;
            Ok((r * 17, g * 17, b * 17))
        }
        _ => Err(RenderError::InvalidColor(s.to_string())),
    }
}

/// Font tokens pre-created from the document before page borrowing.
pub(crate) struct FontTokens {
    pub(crate) helvetica: PdfFontToken,
    pub(crate) helvetica_bold: PdfFontToken,
    pub(crate) helvetica_oblique: PdfFontToken,
}

/// pdfium-render crate series this binary was compiled against.
const PDFIUM_RENDER_VERSION: &str = "0.8";

/// Assert pdfium FFI ABI compatibility at startup.
///
/// Panics with an actionable message if the packaged libpdfium.so cannot be
/// bound. Also exercises FPDF_LoadMemDocument64 so a symbol-resolution or
/// calling-convention mismatch crashes here (startup) instead of on the first
/// annotation request in prod. Skips silently when PDFIUM_LIB_PATH is unset
/// (dev environments without a pinned binary).
pub fn assert_pdfium_abi() {
    let lib_path = match std::env::var("PDFIUM_LIB_PATH") {
        Ok(p) => p,
        Err(_) => return,
    };

    tracing::info!(pdfium_lib = %lib_path, "pdfium ABI canary: binding library");

    let pdfium = match create_pdfium() {
        Ok(p) => p,
        Err(e) => panic!(
            "pdfium ABI canary FAILED: could not bind to libpdfium.so\n\
             Expected: pdfium-render v{PDFIUM_RENDER_VERSION} compatible binary\n\
             PDFIUM_LIB_PATH: {lib_path}\n\
             Error: {e}\n\
             Fix: rebuild the container image with a libpdfium.so \
             compatible with pdfium-render v{PDFIUM_RENDER_VERSION}"
        ),
    };

    // Exercise FPDF_LoadMemDocument64 through the FFI binding. An ABI mismatch
    // would cause a SIGSEGV here (startup) instead of on the first real request.
    // The result is intentionally ignored — parse failure is not an ABI failure.
    tracing::info!("pdfium ABI canary: exercising FFI surface");
    let _ = pdfium.load_pdf_from_byte_slice(b"%PDF-1.4\n%%EOF", None);

    tracing::info!(
        pdfium_lib = %lib_path,
        pdfium_render_version = PDFIUM_RENDER_VERSION,
        "pdfium ABI canary OK"
    );
}

/// Render annotations onto a PDF document, returning the modified PDF bytes.
pub fn render_annotations(
    pdf_bytes: &[u8],
    annotations: &[Annotation],
) -> Result<Vec<u8>, RenderError> {
    validate_pdf(pdf_bytes)?;

    let pdfium = create_pdfium()?;
    let mut document = pdfium.load_pdf_from_byte_slice(pdf_bytes, None)?;
    let page_count = document.pages().len() as u32;

    // Pre-create font tokens (Copy, no lifetime) before borrowing pages.
    let fonts = {
        let fm = document.fonts_mut();
        FontTokens {
            helvetica: fm.helvetica(),
            helvetica_bold: fm.helvetica_bold(),
            helvetica_oblique: fm.helvetica_oblique(),
        }
    };

    let mut by_page: std::collections::HashMap<u32, Vec<&Annotation>> =
        std::collections::HashMap::new();
    for ann in annotations {
        by_page.entry(ann.page_number).or_default().push(ann);
    }

    for (page_num, page_annotations) in &by_page {
        if *page_num < 1 || *page_num > page_count {
            return Err(RenderError::InvalidPage(*page_num, page_count));
        }
        let page_index = (*page_num - 1) as u16;
        let mut page = document.pages_mut().get(page_index)?;
        let page_height = page.height().value;

        for ann in page_annotations {
            let pdf_y = page_height - ann.y;
            match ann.annotation_type {
                AnnotationType::Text | AnnotationType::Callout => {
                    renderers::render_text(&mut page, &fonts, ann, pdf_y)?;
                }
                AnnotationType::Arrow => {
                    renderers::render_arrow(&mut page, ann, page_height)?;
                }
                AnnotationType::Highlight => {
                    renderers::render_highlight(&mut page, ann, page_height)?;
                }
                AnnotationType::Stamp => {
                    renderers::render_stamp(&mut page, &fonts, ann, pdf_y)?;
                }
                AnnotationType::Shape => {
                    renderers::render_shape(&mut page, ann, page_height)?;
                }
                AnnotationType::Freehand => {
                    renderers::render_freehand(&mut page, ann, page_height)?;
                }
                AnnotationType::Bubble => {
                    renderers::render_bubble(&mut page, &fonts, ann, pdf_y)?;
                }
                AnnotationType::Signature => {
                    renderers::render_signature(&mut page, &fonts, ann, pdf_y)?;
                }
                AnnotationType::Whiteout => {
                    renderers::render_whiteout(&mut page, ann, page_height)?;
                }
            }
        }
    }

    Ok(document.save_to_bytes()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdfium_abi_canary() {
        // In dev (PDFIUM_LIB_PATH unset) the canary returns early — correct behavior.
        // In CI containers with PDFIUM_LIB_PATH set, this exercises the live binding.
        assert_pdfium_abi();
    }

    #[test]
    fn test_validate_pdf_magic() {
        assert!(validate_pdf(b"%PDF-1.7 test").is_ok());
        assert!(matches!(
            validate_pdf(b"not a pdf"),
            Err(RenderError::InvalidMagic)
        ));
    }

    #[test]
    fn test_validate_pdf_too_large() {
        let mut data = b"%PDF-".to_vec();
        data.resize(MAX_PDF_SIZE + 1, 0);
        assert!(matches!(validate_pdf(&data), Err(RenderError::TooLarge)));
    }

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#FF0000").unwrap(), (255, 0, 0));
        assert_eq!(parse_hex_color("#00ff00").unwrap(), (0, 255, 0));
        assert_eq!(parse_hex_color("F00").unwrap(), (255, 0, 0));
        assert!(parse_hex_color("invalid").is_err());
    }
}
