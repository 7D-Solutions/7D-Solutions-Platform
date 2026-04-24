//! Coordinate round-trip harness (bd-87uqu).
//!
//! Tests that annotation coordinates survive the frontend→wire→Rust→render pipeline
//! without unit mismatches (normalized vs absolute, pixels vs points, Y-axis origin).
//!
//! Strategy: deserialize a fixture, verify the coord transform math produces the
//! expected PDF-space values within 0.5 PDF-point tolerance, then re-serialize and
//! confirm wire shape is preserved.
//!
//! Coverage: FREEHAND, BUBBLE, SIGNATURE (DRAW), ARROW, CALLOUT, SHAPE (RECT).

use pdf_editor::domain::annotations::types::{Annotation, AnnotationType};

const TOLERANCE_PT: f32 = 0.5;

fn assert_within(label: &str, got: f32, expected: f32) {
    let delta = (got - expected).abs();
    assert!(
        delta <= TOLERANCE_PT,
        "{}: got {}, expected {} (delta {:.3} > tolerance {})",
        label,
        got,
        expected,
        delta,
        TOLERANCE_PT
    );
}

/// Canvas→PDF Y transform: pdf_y = page_height - canvas_y
fn canvas_to_pdf_y(canvas_y: f32, page_height: f32) -> f32 {
    page_height - canvas_y
}

/// Deserialize a fixture JSON string into Annotation.
fn parse(json: &str) -> Annotation {
    serde_json::from_str(json).unwrap_or_else(|e| panic!("parse failed: {e}\nJSON: {json}"))
}

/// Serialize and deserialize — confirms the shape round-trips without data loss.
fn round_trip(ann: &Annotation) -> Annotation {
    let s = serde_json::to_string(ann).expect("serialize");
    serde_json::from_str(&s).expect("deserialize round-trip")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn coord_round_trip_text_anchor() {
    let json = r#"{"schemaVersion":1,"id":"rt-text","type":"TEXT","x":72.0,"y":144.0,"pageNumber":1,"text":"hello"}"#;
    let ann = parse(json);

    assert_eq!(ann.annotation_type, AnnotationType::Text);
    assert_within("x", ann.x, 72.0);
    assert_within("y", ann.y, 144.0);

    // PDF-space Y = page_height - canvas_y
    let page_height = 792.0_f32; // US Letter in points
    let pdf_y = canvas_to_pdf_y(ann.y, page_height);
    assert_within("pdf_y", pdf_y, 648.0);

    let rt = round_trip(&ann);
    assert_within("rt.x", rt.x, ann.x);
    assert_within("rt.y", rt.y, ann.y);
}

#[test]
fn coord_round_trip_arrow_endpoints() {
    let json = r#"{"schemaVersion":1,"id":"rt-arrow","type":"ARROW","x":50.0,"y":100.0,"x2":250.0,"y2":300.0,"pageNumber":1}"#;
    let ann = parse(json);

    assert_eq!(ann.annotation_type, AnnotationType::Arrow);
    assert_within("x", ann.x, 50.0);
    assert_within("y", ann.y, 100.0);
    assert_within("x2", ann.x2.unwrap(), 250.0);
    assert_within("y2", ann.y2.unwrap(), 300.0);

    let page_height = 792.0;
    assert_within("pdf_y_start", canvas_to_pdf_y(ann.y, page_height), 692.0);
    assert_within("pdf_y_end", canvas_to_pdf_y(ann.y2.unwrap(), page_height), 492.0);

    let rt = round_trip(&ann);
    assert_within("rt.x2", rt.x2.unwrap(), ann.x2.unwrap());
}

#[test]
fn coord_round_trip_shape_bounding_box() {
    let json = r#"{"schemaVersion":1,"id":"rt-rect","type":"SHAPE","x":80.0,"y":90.0,"pageNumber":1,"shapeType":"RECTANGLE","width":150.0,"height":80.0}"#;
    let ann = parse(json);

    assert_eq!(ann.annotation_type, AnnotationType::Shape);
    assert_within("x", ann.x, 80.0);
    assert_within("y", ann.y, 90.0);
    assert_within("w", ann.width.unwrap(), 150.0);
    assert_within("h", ann.height.unwrap(), 80.0);

    // Bottom-right in canvas space
    let br_x = ann.x + ann.width.unwrap();
    let br_y = ann.y + ann.height.unwrap();
    assert_within("br_x", br_x, 230.0);
    assert_within("br_y", br_y, 170.0);

    let page_height = 792.0;
    // In PDF space, top-left of rect is pdf_y = page_height - ann.y
    // bottom-right = pdf_y - height
    let pdf_top = canvas_to_pdf_y(ann.y, page_height);
    let pdf_bottom = pdf_top - ann.height.unwrap();
    assert_within("pdf_top", pdf_top, 702.0);
    assert_within("pdf_bottom", pdf_bottom, 622.0);

    let rt = round_trip(&ann);
    assert_within("rt.width", rt.width.unwrap(), ann.width.unwrap());
    assert_within("rt.height", rt.height.unwrap(), ann.height.unwrap());
}

#[test]
fn coord_round_trip_freehand_path() {
    let json = r#"{"schemaVersion":1,"id":"rt-free","type":"FREEHAND","x":0.0,"y":0.0,"pageNumber":1,"path":[{"x":10.0,"y":20.0},{"x":30.0,"y":40.0},{"x":50.0,"y":60.0}]}"#;
    let ann = parse(json);

    assert_eq!(ann.annotation_type, AnnotationType::Freehand);
    let path = ann.path.as_ref().expect("path");
    assert_eq!(path.len(), 3);
    assert_within("path[0].x", path[0].x, 10.0);
    assert_within("path[0].y", path[0].y, 20.0);
    assert_within("path[2].x", path[2].x, 50.0);
    assert_within("path[2].y", path[2].y, 60.0);

    let page_height = 792.0;
    // Freehand path coords are absolute PDF-space from the annotation anchor
    let pdf_y_0 = canvas_to_pdf_y(path[0].y, page_height);
    assert_within("pdf_path[0].y", pdf_y_0, 772.0);

    let rt = round_trip(&ann);
    let rt_path = rt.path.as_ref().expect("rt path");
    assert_within("rt.path[1].x", rt_path[1].x, path[1].x);
}

#[test]
fn coord_round_trip_signature_draw_normalized() {
    // Signature path coords are 0..1 NORMALIZED relative to the bounding box.
    let json = r#"{"schemaVersion":1,"id":"rt-sig","type":"SIGNATURE","x":100.0,"y":600.0,"pageNumber":1,"width":200.0,"height":80.0,"signatureMethod":"DRAW","signaturePath":[{"x":0.1,"y":0.2,"newStroke":true},{"x":0.5,"y":0.4},{"x":0.9,"y":0.2}]}"#;
    let ann = parse(json);

    assert_eq!(ann.annotation_type, AnnotationType::Signature);
    let sig_path = ann.signature_path.as_ref().expect("signature path");
    assert_eq!(sig_path.len(), 3);

    // Normalized 0..1 coords — should be in [0,1] range
    for pt in sig_path {
        assert!(pt.x >= 0.0 && pt.x <= 1.0, "sig_path x must be normalized 0..1");
        assert!(pt.y >= 0.0 && pt.y <= 1.0, "sig_path y must be normalized 0..1");
    }

    let w = ann.width.unwrap_or(0.0);
    let h = ann.height.unwrap_or(0.0);

    // Reconstruct absolute canvas coords: anchor + normalized * dimension
    let abs_x0 = ann.x + sig_path[0].x * w;
    let abs_y0 = ann.y + sig_path[0].y * h;
    assert_within("abs_x0", abs_x0, 120.0); // 100 + 0.1*200
    assert_within("abs_y0", abs_y0, 616.0); // 600 + 0.2*80

    let rt = round_trip(&ann);
    let rt_path = rt.signature_path.as_ref().expect("rt sig path");
    assert_within("rt sig_path[0].x", rt_path[0].x, sig_path[0].x);

    // Verify newStroke survived the round-trip
    assert_eq!(rt_path[0].new_stroke, sig_path[0].new_stroke);
}

#[test]
fn coord_round_trip_bubble() {
    let json = r#"{"schemaVersion":1,"id":"rt-bubble","type":"BUBBLE","x":200.0,"y":350.0,"pageNumber":2,"bubbleNumber":3,"bubbleSize":30.0,"hasLeaderLine":true,"leaderX":220.0,"leaderY":380.0}"#;
    let ann = parse(json);

    assert_eq!(ann.annotation_type, AnnotationType::Bubble);
    assert_within("x", ann.x, 200.0);
    assert_within("y", ann.y, 350.0);
    assert_within("leaderX", ann.leader_x.unwrap(), 220.0);
    assert_within("leaderY", ann.leader_y.unwrap(), 380.0);

    let page_height = 841.89; // A4 in points
    let pdf_y = canvas_to_pdf_y(ann.y, page_height);
    let pdf_leader_y = canvas_to_pdf_y(ann.leader_y.unwrap(), page_height);
    assert!(pdf_y > pdf_leader_y, "anchor must be above leader in PDF space");

    let rt = round_trip(&ann);
    assert_eq!(rt.has_leader_line, ann.has_leader_line);
    assert_within("rt.leaderX", rt.leader_x.unwrap(), ann.leader_x.unwrap());
}

#[test]
fn coord_round_trip_callout() {
    let json = r#"{"schemaVersion":1,"id":"rt-callout","type":"CALLOUT","x":300.0,"y":250.0,"pageNumber":1,"text":"Callout text","width":120.0,"height":60.0}"#;
    let ann = parse(json);

    assert_eq!(ann.annotation_type, AnnotationType::Callout);
    assert_within("x", ann.x, 300.0);
    assert_within("y", ann.y, 250.0);
    assert_within("w", ann.width.unwrap(), 120.0);
    assert_within("h", ann.height.unwrap(), 60.0);

    let rt = round_trip(&ann);
    assert_eq!(rt.text, ann.text);
    assert_within("rt.width", rt.width.unwrap(), ann.width.unwrap());
}
