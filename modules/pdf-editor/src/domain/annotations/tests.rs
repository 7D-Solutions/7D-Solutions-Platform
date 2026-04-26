use super::renderers::{arrow_geometry, callout_edge_point, leader_geometry};
use super::types::{Annotation, BubbleShape, CURRENT_ANNOTATION_SCHEMA_VERSION};

#[test]
fn annotation_schema_version_explicit_integer() {
    let json = r#"{"id":"t1","x":1.0,"y":2.0,"pageNumber":1,"type":"TEXT","schemaVersion":1}"#;
    let ann: Annotation = serde_json::from_str(json).expect("valid annotation JSON");
    assert_eq!(ann.schema_version, 1);
    ann.validate_schema_version().expect("version 1 must be valid");
}

#[test]
fn annotation_schema_version_absent_defaults_to_current() {
    let json = r#"{"id":"t1","x":1.0,"y":2.0,"pageNumber":1,"type":"TEXT"}"#;
    let ann: Annotation = serde_json::from_str(json).expect("valid annotation JSON");
    assert_eq!(ann.schema_version, CURRENT_ANNOTATION_SCHEMA_VERSION);
    ann.validate_schema_version().expect("default version must be valid");
}

#[test]
fn annotation_schema_version_snake_case_wire_name_ignored() {
    // "schema_version" (snake_case) is NOT the wire name — "schemaVersion" (camelCase) is.
    // serde ignores unknown keys, so schema_version stays at the default.
    let json = r#"{"id":"t1","x":1.0,"y":2.0,"pageNumber":1,"type":"TEXT","schema_version":99}"#;
    let ann: Annotation = serde_json::from_str(json).expect("valid annotation JSON");
    assert_eq!(ann.schema_version, CURRENT_ANNOTATION_SCHEMA_VERSION);
}

#[test]
fn annotation_schema_version_unsupported_returns_error() {
    // schema_version=99 is out of range — validate_schema_version must return Err
    let json = r#"{"id":"t1","x":1.0,"y":2.0,"pageNumber":1,"type":"TEXT","schemaVersion":99}"#;
    let ann: Annotation = serde_json::from_str(json).expect("valid annotation JSON");
    assert_eq!(ann.schema_version, 99);
    let err = ann.validate_schema_version().unwrap_err();
    assert_eq!(err.version, 99);
    assert!(err.to_string().contains("unsupported annotation schema_version=99"));
}

#[test]
fn bubble_shape_round_trips_all_variants() {
    for (wire, expected) in [
        ("\"CIRCLE\"", BubbleShape::Circle),
        ("\"SQUARE\"", BubbleShape::Square),
        ("\"OVAL\"", BubbleShape::Oval),
    ] {
        let json = format!(
            r#"{{"id":"b1","x":10.0,"y":20.0,"pageNumber":1,"type":"BUBBLE","bubbleShape":{}}}"#,
            wire
        );
        let ann: Annotation = serde_json::from_str(&json).expect("valid bubble annotation JSON");
        assert_eq!(ann.bubble_shape.as_ref().expect("bubble_shape parsed"), &expected);
        let re = serde_json::to_string(&ann).expect("annotation serializes");
        assert!(re.contains(wire), "re-serialized JSON must contain {wire}");
    }
}

#[test]
fn bubble_shape_absent_deserializes_to_none() {
    let json = r#"{"id":"b2","x":5.0,"y":5.0,"pageNumber":1,"type":"BUBBLE"}"#;
    let ann: Annotation = serde_json::from_str(json).expect("valid bubble annotation JSON");
    assert!(ann.bubble_shape.is_none());
}

// ── leader_geometry golden tests ──────────────────────────────────────────────
//
// All inputs are in screen space (y=0 top, increases downward).
// Expected outputs are in PDF space (y=0 bottom, increases upward).
//
// Formula:
//   origin_x = anchor_x + radius
//   origin_y = page_height - anchor_y - radius
//   target_x = leader_x
//   target_y = page_height - leader_y

#[test]
fn leader_geometry_standard_bubble_below_anchor() {
    // anchor at (100, 200), leader target at (50, 300), bubble_size=24, page_height=792
    // radius=12, origin=(112, 580), target=(50, 492)
    let (ox, oy, tx, ty) = leader_geometry(100.0, 200.0, 50.0, 300.0, 24.0, 792.0);
    assert_eq!(ox, 112.0);
    assert_eq!(oy, 580.0);
    assert_eq!(tx, 50.0);
    assert_eq!(ty, 492.0);
}

#[test]
fn leader_geometry_target_above_anchor() {
    // leader target is above the bubble in screen space (lower leader_y)
    // anchor=(300, 400), leader=(150, 200), bubble_size=32, page_height=841.89
    // radius=16, origin=(316, 425.89), target=(150, 641.89)
    let (ox, oy, tx, ty) = leader_geometry(300.0, 400.0, 150.0, 200.0, 32.0, 841.89);
    assert!((ox - 316.0).abs() < 1e-3);
    assert!((oy - 425.89).abs() < 1e-3);
    assert!((tx - 150.0).abs() < 1e-3);
    assert!((ty - 641.89).abs() < 1e-3);
}

#[test]
fn leader_geometry_origin_center_varies_with_bubble_size() {
    // Changing bubble_size shifts origin but not target
    let (ox_small, oy_small, tx_small, ty_small) =
        leader_geometry(100.0, 100.0, 200.0, 200.0, 20.0, 792.0);
    let (ox_large, oy_large, tx_large, ty_large) =
        leader_geometry(100.0, 100.0, 200.0, 200.0, 40.0, 792.0);

    // target is independent of bubble size
    assert_eq!(tx_small, tx_large);
    assert_eq!(ty_small, ty_large);

    // origin shifts by half the size difference (10 points)
    assert!((ox_large - ox_small - 10.0).abs() < 1e-4);
    assert!((oy_small - oy_large - 10.0).abs() < 1e-4);
}

#[test]
fn leader_geometry_anchor_at_page_origin() {
    // anchor at (0, 0) — top-left corner of page in screen space
    // bubble_size=24, page_height=792
    // radius=12, origin=(12, 780), target=(0, 792)
    let (ox, oy, tx, ty) = leader_geometry(0.0, 0.0, 0.0, 0.0, 24.0, 792.0);
    assert_eq!(ox, 12.0);
    assert_eq!(oy, 780.0);
    assert_eq!(tx, 0.0);
    assert_eq!(ty, 792.0);
}

#[test]
fn leader_geometry_target_coincides_with_origin() {
    // leader target at exact center of bubble — degenerate zero-length line is valid
    // anchor=(100, 200), bubble_size=24, page_height=792
    // center in screen space: (112, 212); in PDF space: (112, 580)
    let (ox, oy, tx, ty) = leader_geometry(100.0, 200.0, 112.0, 212.0, 24.0, 792.0);
    assert_eq!(ox, tx);
    assert_eq!(oy, ty);
}

// ── arrow_geometry golden tests ───────────────────────────────────────────────
//
// All values are in screen space (y=0 top, y increases downward).
// Locked conventions:
//   - Spread factor = 0.4  (half-angle ≈ 21.8°, total opening ≈ 43.6°)
//   - barb_x = tip_x - head_size * (ux ± 0.4*uy)
//   - barb_y = tip_y - head_size * (uy ∓ 0.4*ux)
//
// Rightward arrow (0,0)→(100,0), head_size=10:
//   ux=1 uy=0 → barb1=(90, 4), barb2=(90, -4)
//
// Downward arrow in screen space (0,0)→(0,100), head_size=10:
//   ux=0 uy=1 → barb1=(-4, 90), barb2=(4, 90)

#[test]
fn arrow_geometry_rightward_shaft() {
    // Horizontal arrow pointing right; barbs symmetric about shaft
    let (bx1, by1, bx2, by2) = arrow_geometry(0.0, 0.0, 100.0, 0.0, 10.0);
    assert_eq!(bx1, 90.0);
    assert_eq!(by1, 4.0);
    assert_eq!(bx2, 90.0);
    assert_eq!(by2, -4.0);
}

#[test]
fn arrow_geometry_downward_shaft_screen_space() {
    // Arrow pointing down in screen space; barbs symmetric about shaft
    let (bx1, by1, bx2, by2) = arrow_geometry(0.0, 0.0, 0.0, 100.0, 10.0);
    assert_eq!(bx1, -4.0);
    assert_eq!(by1, 90.0);
    assert_eq!(bx2, 4.0);
    assert_eq!(by2, 90.0);
}

#[test]
fn arrow_geometry_barbs_symmetric_about_shaft() {
    // Barbs must be mirror images across the shaft axis
    // Using a rightward arrow: barb_y values are equal magnitude, opposite sign
    let (bx1, by1, bx2, by2) = arrow_geometry(50.0, 200.0, 150.0, 200.0, 12.0);
    assert_eq!(bx1, bx2, "barb x coords must be equal for horizontal shaft");
    assert!((by1 + by2 - 400.0).abs() < 1e-4, "barbs symmetric about shaft y={}", 200.0);
    assert!((by1 - by2).abs() > 0.0, "barbs must be offset from shaft");
}

#[test]
fn arrow_geometry_head_size_scales_proportionally() {
    // Doubling head_size doubles the barb distance from the tip
    let (bx1_s, by1_s, bx2_s, by2_s) = arrow_geometry(0.0, 0.0, 100.0, 0.0, 10.0);
    let (bx1_d, by1_d, bx2_d, by2_d) = arrow_geometry(0.0, 0.0, 100.0, 0.0, 20.0);
    assert!((bx1_d - 100.0 - 2.0 * (bx1_s - 100.0)).abs() < 1e-4);
    assert!((by1_d - 2.0 * by1_s).abs() < 1e-4);
    assert!((bx2_d - 100.0 - 2.0 * (bx2_s - 100.0)).abs() < 1e-4);
    assert!((by2_d - 2.0 * by2_s).abs() < 1e-4);
}

#[test]
fn arrow_geometry_zero_length_shaft_no_panic() {
    // Degenerate case: tail == tip. Must not panic; barbs collapse to tip.
    let (bx1, by1, bx2, by2) = arrow_geometry(50.0, 50.0, 50.0, 50.0, 10.0);
    assert!(bx1.is_finite());
    assert!(by1.is_finite());
    assert!(bx2.is_finite());
    assert!(by2.is_finite());
}

// ── callout_edge_point golden tests ───────────────────────────────────────────
//
// All inputs are screen space (y=0 top, increases downward).
// Expected outputs are also screen space — the caller converts to PDF space.
//
// Box: x=10, y=20, w=120, h=40 → center=(70, 40), half_w=60, half_h=20

#[test]
fn callout_render_edge_point_below_box() {
    // Leader target directly below center → exits through bottom-center edge
    // center=(70,40), target=(70,100): dx=0, dy=60 → t=20/60, edge=(70,60)
    let (ex, ey) = callout_edge_point(70.0, 40.0, 60.0, 20.0, 70.0, 100.0);
    assert!((ex - 70.0).abs() < 1e-4);
    assert!((ey - 60.0).abs() < 1e-4);
}

#[test]
fn callout_render_edge_point_right_of_box() {
    // Leader target directly right of center → exits through right-center edge
    // center=(70,40), target=(200,40): dx=130, dy=0 → t=60/130, edge=(130,40)
    let (ex, ey) = callout_edge_point(70.0, 40.0, 60.0, 20.0, 200.0, 40.0);
    assert!((ex - 130.0).abs() < 1e-4);
    assert!((ey - 40.0).abs() < 1e-4);
}

#[test]
fn callout_render_edge_point_diagonal_corner() {
    // Leader target at diagonal where both axes clip simultaneously → corner
    // center=(50,30), half_w=50, half_h=30, target=(150,120)
    // dx=100, dy=90 → t_x=50/100=0.5, t_y=30/90≈0.333 → t≈0.333
    // edge=(50+33.3, 30+30) = (83.3, 60)
    let (ex, ey) = callout_edge_point(50.0, 30.0, 50.0, 30.0, 150.0, 120.0);
    assert!((ey - 60.0).abs() < 1e-3);
    assert!(ex > 50.0 && ex < 100.0);
}

#[test]
fn callout_render_edge_point_degenerate_target_at_center() {
    // Degenerate: target == center → returns bottom-center
    let (ex, ey) = callout_edge_point(70.0, 40.0, 60.0, 20.0, 70.0, 40.0);
    assert_eq!(ex, 70.0);
    assert_eq!(ey, 60.0);
}
