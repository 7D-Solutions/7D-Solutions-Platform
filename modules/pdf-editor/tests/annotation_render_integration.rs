//! Integration tests for annotation rendering (bd-1647).
//!
//! Tests the actual pdfium-render pipeline: real PDF bytes in → annotated PDF bytes out.
//! Requires libpdfium.dylib (macOS) or libpdfium.so (Linux) in the module directory
//! or PDFIUM_LIB_PATH set.

use pdf_editor::domain::annotations::{
    render::{render_annotations, validate_pdf, RenderError, MAX_PDF_SIZE},
    types::{Annotation, AnnotationType, Point, ShapeType, SignaturePoint, StampType, TextRect},
};

/// Build a minimal annotation with only the required fields set.
fn base_annotation(ann_type: AnnotationType, page: u32) -> Annotation {
    Annotation {
        id: uuid::Uuid::new_v4().to_string(),
        x: 100.0,
        y: 100.0,
        page_number: page,
        annotation_type: ann_type,
        text: None,
        font_size: None,
        font_family: None,
        font_weight: None,
        font_style: None,
        color: None,
        bg_color: None,
        border_color: None,
        x2: None,
        y2: None,
        arrowhead_size: None,
        stroke_width: None,
        shape_type: None,
        width: None,
        height: None,
        opacity: None,
        text_rects: None,
        stamp_type: None,
        stamp_username: None,
        stamp_date: None,
        stamp_time: None,
        path: None,
        bubble_number: None,
        bubble_size: None,
        bubble_color: None,
        bubble_border_color: None,
        text_color: None,
        bubble_font_size: None,
        bubble_shape: None,
        has_leader_line: None,
        leader_x: None,
        leader_y: None,
        leader_color: None,
        leader_stroke_width: None,
        signature_method: None,
        signature_path: None,
        signature_image: None,
        signature_text: None,
        schema_version: 1,
    }
}

fn test_pdf_bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/test.pdf"
    ))
    .expect("test.pdf fixture not found")
}

// ============================================================================
// Validation tests
// ============================================================================

#[test]
fn validate_pdf_rejects_non_pdf() {
    let result = validate_pdf(b"not a pdf file");
    assert!(matches!(result, Err(RenderError::InvalidMagic)));
}

#[test]
fn validate_pdf_rejects_oversized() {
    let mut data = b"%PDF-".to_vec();
    data.resize(MAX_PDF_SIZE + 1, 0);
    assert!(matches!(validate_pdf(&data), Err(RenderError::TooLarge)));
}

#[test]
fn validate_pdf_accepts_valid() {
    let pdf = test_pdf_bytes();
    assert!(validate_pdf(&pdf).is_ok());
}

// ============================================================================
// Render tests — each annotation type
// ============================================================================

#[test]
fn render_text_annotation() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Text, 1);
    ann.text = Some("Hello World".to_string());
    ann.font_size = Some(16.0);
    ann.color = Some("#000000".to_string());

    let result = render_annotations(&pdf, &[ann]);
    assert!(result.is_ok(), "render_text failed: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.starts_with(b"%PDF-"), "output is not valid PDF");
    assert!(
        output.len() > pdf.len(),
        "output should be larger than input"
    );
}

#[test]
fn render_arrow_annotation() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Arrow, 1);
    ann.x2 = Some(300.0);
    ann.y2 = Some(200.0);
    ann.stroke_width = Some(3.0);
    ann.color = Some("#FF0000".to_string());
    ann.arrowhead_size = Some(12.0);

    let result = render_annotations(&pdf, &[ann]);
    assert!(result.is_ok(), "render_arrow failed: {:?}", result.err());
    assert!(result.unwrap().starts_with(b"%PDF-"));
}

#[test]
fn render_highlight_with_text_rects() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Highlight, 1);
    ann.color = Some("#FFFF00".to_string());
    ann.opacity = Some(0.4);
    ann.text_rects = Some(vec![
        TextRect {
            x: 50.0,
            y: 100.0,
            width: 200.0,
            height: 14.0,
        },
        TextRect {
            x: 50.0,
            y: 116.0,
            width: 150.0,
            height: 14.0,
        },
    ]);

    let result = render_annotations(&pdf, &[ann]);
    assert!(
        result.is_ok(),
        "render_highlight failed: {:?}",
        result.err()
    );
    assert!(result.unwrap().starts_with(b"%PDF-"));
}

#[test]
fn render_highlight_fallback_rect() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Highlight, 1);
    ann.width = Some(200.0);
    ann.height = Some(20.0);
    ann.color = Some("#00FF00".to_string());

    let result = render_annotations(&pdf, &[ann]);
    assert!(
        result.is_ok(),
        "render_highlight fallback failed: {:?}",
        result.err()
    );
}

#[test]
fn render_stamp_approved() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Stamp, 1);
    ann.stamp_type = Some(StampType::Approved);
    ann.stamp_username = Some("John Doe".to_string());
    ann.stamp_date = Some("2026-02-24".to_string());
    ann.stamp_time = Some("14:30".to_string());
    ann.width = Some(160.0);
    ann.height = Some(50.0);
    ann.font_size = Some(14.0);
    ann.color = Some("#008000".to_string());
    ann.bg_color = Some("#FFFFFF".to_string());
    ann.border_color = Some("#008000".to_string());

    let result = render_annotations(&pdf, &[ann]);
    assert!(result.is_ok(), "render_stamp failed: {:?}", result.err());
    assert!(result.unwrap().starts_with(b"%PDF-"));
}

#[test]
fn render_stamp_custom() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Stamp, 1);
    ann.stamp_type = Some(StampType::Custom);
    ann.text = Some("RUSH ORDER".to_string());

    let result = render_annotations(&pdf, &[ann]);
    assert!(
        result.is_ok(),
        "render_stamp_custom failed: {:?}",
        result.err()
    );
}

#[test]
fn render_shape_rectangle() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Shape, 1);
    ann.shape_type = Some(ShapeType::Rectangle);
    ann.width = Some(120.0);
    ann.height = Some(80.0);
    ann.border_color = Some("#0000FF".to_string());
    ann.stroke_width = Some(2.0);

    let result = render_annotations(&pdf, &[ann]);
    assert!(
        result.is_ok(),
        "render_shape_rect failed: {:?}",
        result.err()
    );
}

#[test]
fn render_shape_circle() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Shape, 1);
    ann.shape_type = Some(ShapeType::Circle);
    ann.width = Some(80.0);
    ann.height = Some(80.0);
    ann.border_color = Some("#FF00FF".to_string());
    ann.bg_color = Some("#FFCCFF".to_string());

    let result = render_annotations(&pdf, &[ann]);
    assert!(
        result.is_ok(),
        "render_shape_circle failed: {:?}",
        result.err()
    );
}

#[test]
fn render_shape_line() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Shape, 1);
    ann.shape_type = Some(ShapeType::Line);
    ann.x2 = Some(400.0);
    ann.y2 = Some(500.0);

    let result = render_annotations(&pdf, &[ann]);
    assert!(
        result.is_ok(),
        "render_shape_line failed: {:?}",
        result.err()
    );
}

#[test]
fn render_freehand_annotation() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Freehand, 1);
    ann.path = Some(vec![
        Point { x: 100.0, y: 100.0 },
        Point { x: 110.0, y: 105.0 },
        Point { x: 120.0, y: 110.0 },
        Point { x: 130.0, y: 108.0 },
        Point { x: 140.0, y: 100.0 },
    ]);
    ann.stroke_width = Some(2.0);
    ann.color = Some("#000000".to_string());

    let result = render_annotations(&pdf, &[ann]);
    assert!(result.is_ok(), "render_freehand failed: {:?}", result.err());
    assert!(result.unwrap().starts_with(b"%PDF-"));
}

#[test]
fn render_freehand_too_few_points_is_noop() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Freehand, 1);
    ann.path = Some(vec![Point { x: 100.0, y: 100.0 }]);

    let result = render_annotations(&pdf, &[ann]);
    assert!(result.is_ok());
}

#[test]
fn render_bubble_annotation() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Bubble, 1);
    ann.bubble_number = Some(42);
    ann.bubble_size = Some(30.0);
    ann.bubble_color = Some("#FF0000".to_string());
    ann.bubble_border_color = Some("#000000".to_string());
    ann.text_color = Some("#FFFFFF".to_string());
    ann.bubble_font_size = Some(14.0);

    let result = render_annotations(&pdf, &[ann]);
    assert!(result.is_ok(), "render_bubble failed: {:?}", result.err());
    assert!(result.unwrap().starts_with(b"%PDF-"));
}

#[test]
fn render_bubble_with_leader_line() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Bubble, 1);
    ann.bubble_number = Some(1);
    ann.bubble_size = Some(24.0);
    ann.has_leader_line = Some(true);
    ann.leader_x = Some(200.0);
    ann.leader_y = Some(300.0);

    let result = render_annotations(&pdf, &[ann]);
    assert!(
        result.is_ok(),
        "render_bubble_leader failed: {:?}",
        result.err()
    );
}

#[test]
fn render_signature_text() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Signature, 1);
    ann.signature_method = Some("TEXT".to_string());
    ann.signature_text = Some("Jane Smith".to_string());
    ann.font_size = Some(18.0);
    ann.color = Some("#000080".to_string());

    let result = render_annotations(&pdf, &[ann]);
    assert!(
        result.is_ok(),
        "render_signature_text failed: {:?}",
        result.err()
    );
    assert!(result.unwrap().starts_with(b"%PDF-"));
}

#[test]
fn render_signature_draw() {
    // signature_path coords are 0..1 normalized relative to width/height.
    // anchor=(100,200), box=50×30, path=[(0.5,0.5),(1.0,1.0)]
    // expected segment: (125, pdf_y-15) → (150, pdf_y-30)
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Signature, 1);
    ann.x = 100.0;
    ann.y = 200.0;
    ann.width = Some(50.0);
    ann.height = Some(30.0);
    ann.signature_method = Some("DRAW".to_string());
    ann.signature_path = Some(vec![
        SignaturePoint {
            x: 0.5,
            y: 0.5,
            new_stroke: None,
        },
        SignaturePoint {
            x: 1.0,
            y: 1.0,
            new_stroke: None,
        },
    ]);
    ann.stroke_width = Some(2.0);
    ann.color = Some("#000000".to_string());

    let result = render_annotations(&pdf, &[ann]);
    assert!(
        result.is_ok(),
        "render_signature_draw failed: {:?}",
        result.err()
    );
    assert!(
        result.unwrap().starts_with(b"%PDF-"),
        "output must be valid PDF"
    );
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn render_empty_annotations_returns_original() {
    let pdf = test_pdf_bytes();
    let result = render_annotations(&pdf, &[]);
    assert!(result.is_ok());
    // With zero annotations, should return equivalent PDF
    assert!(result.unwrap().starts_with(b"%PDF-"));
}

#[test]
fn render_invalid_page_number_rejected() {
    let pdf = test_pdf_bytes();
    let ann = base_annotation(AnnotationType::Text, 99);

    let result = render_annotations(&pdf, &[ann]);
    assert!(
        matches!(result, Err(RenderError::InvalidPage(99, 1))),
        "expected InvalidPage(99, 1), got: {:?}",
        result
    );
}

#[test]
fn render_page_zero_rejected() {
    let pdf = test_pdf_bytes();
    let ann = base_annotation(AnnotationType::Text, 0);

    let result = render_annotations(&pdf, &[ann]);
    assert!(
        matches!(result, Err(RenderError::InvalidPage(0, _))),
        "page 0 should be rejected: {:?}",
        result
    );
}

#[test]
fn render_multiple_annotation_types_on_one_page() {
    let pdf = test_pdf_bytes();
    let mut text = base_annotation(AnnotationType::Text, 1);
    text.text = Some("Title".to_string());
    text.font_size = Some(20.0);

    let mut arrow = base_annotation(AnnotationType::Arrow, 1);
    arrow.x = 200.0;
    arrow.y = 200.0;
    arrow.x2 = Some(350.0);
    arrow.y2 = Some(300.0);
    arrow.color = Some("#FF0000".to_string());

    let mut highlight = base_annotation(AnnotationType::Highlight, 1);
    highlight.x = 50.0;
    highlight.y = 400.0;
    highlight.width = Some(200.0);
    highlight.height = Some(16.0);

    let mut stamp = base_annotation(AnnotationType::Stamp, 1);
    stamp.x = 400.0;
    stamp.y = 50.0;
    stamp.stamp_type = Some(StampType::Reviewed);
    stamp.width = Some(120.0);
    stamp.height = Some(40.0);

    let result = render_annotations(&pdf, &[text, arrow, highlight, stamp]);
    assert!(
        result.is_ok(),
        "mixed annotations failed: {:?}",
        result.err()
    );
    assert!(result.unwrap().starts_with(b"%PDF-"));
}

#[test]
fn render_text_empty_string_is_noop() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Text, 1);
    ann.text = Some(String::new());

    let result = render_annotations(&pdf, &[ann]);
    assert!(result.is_ok());
}

#[test]
fn render_non_pdf_bytes_rejected() {
    let ann = base_annotation(AnnotationType::Text, 1);
    let result = render_annotations(b"not a pdf at all", &[ann]);
    assert!(matches!(result, Err(RenderError::InvalidMagic)));
}

#[test]
fn render_callout_uses_text_path() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Callout, 1);
    ann.text = Some("Important note".to_string());
    ann.font_size = Some(12.0);

    let result = render_annotations(&pdf, &[ann]);
    assert!(result.is_ok(), "render_callout failed: {:?}", result.err());
}

#[test]
fn render_default_colors_when_none_specified() {
    let pdf = test_pdf_bytes();
    let mut ann = base_annotation(AnnotationType::Text, 1);
    ann.text = Some("No color specified".to_string());
    // No color set — should use default black

    let result = render_annotations(&pdf, &[ann]);
    assert!(result.is_ok(), "default colors failed: {:?}", result.err());
}
