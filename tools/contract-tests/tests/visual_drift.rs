//! Visual drift tolerance tests (bd-dytrn).
//!
//! Renders each annotation-class fixture onto a base PDF, rasterizes page 1 to a PNG,
//! pixel-diffs against a committed golden snapshot, and fails if the diff ratio exceeds
//! the per-class tolerance.
//!
//! Tolerance by class:
//!   FREEHAND / SIGNATURE              — 2.0 %  (rasterization legitimately noisier)
//!   BUBBLE / CALLOUT / ARROW / SHAPE
//!     / HIGHLIGHT / WHITEOUT          — 1.0 %  (line-art with minor AA jitter)
//!   TEXT / STAMP                      — 0.5 %  (font / solid-fill must be stable)
//!
//! Update goldens (requires PDFIUM_LIB_PATH):
//!   UPDATE_GOLDENS=1 ./scripts/cargo-slot.sh test -p contract-tests -- visual_drift
//!
//! Requires PDFIUM_LIB_PATH to point at a compatible libpdfium shared library.
//! Tests skip silently when that variable is absent (dev environments without pdfium).

use image::{ImageFormat, RgbaImage};
use pdf_editor::domain::annotations::{render::render_annotations, types::AnnotationType};
use pdfium_render::prelude::*;
use std::path::PathBuf;

// ── Tolerance constants ───────────────────────────────────────────────────────

const TOLERANCE_HIGH: f64 = 0.02; // 2 % — FREEHAND, SIGNATURE
const TOLERANCE_MID: f64 = 0.01;  // 1 % — BUBBLE, CALLOUT, ARROW, SHAPE, HIGHLIGHT, WHITEOUT
const TOLERANCE_LOW: f64 = 0.005; // 0.5 % — TEXT, STAMP

fn class_tolerance(ann_type: AnnotationType) -> f64 {
    match ann_type {
        AnnotationType::Freehand | AnnotationType::Signature => TOLERANCE_HIGH,
        AnnotationType::Bubble
        | AnnotationType::Callout
        | AnnotationType::Arrow
        | AnnotationType::Shape
        | AnnotationType::Highlight
        | AnnotationType::Whiteout => TOLERANCE_MID,
        AnnotationType::Text | AnnotationType::Stamp => TOLERANCE_LOW,
    }
}

// ── Paths ─────────────────────────────────────────────────────────────────────

fn goldens_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("goldens")
}

fn consumer_fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/pdf-editor-consumer")
        .join(format!("{name}.json"))
}

fn base_pdf_bytes() -> Vec<u8> {
    // Navigate from tools/contract-tests to the pdf-editor test fixture.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../modules/pdf-editor/tests/fixtures/test.pdf");
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Base PDF not found at {}: {e}", path.display()))
}

// ── Pdfium setup ──────────────────────────────────────────────────────────────

fn try_create_pdfium() -> Option<Pdfium> {
    let lib_path = std::env::var("PDFIUM_LIB_PATH").ok()?;
    let bindings = Pdfium::bind_to_library(&lib_path).ok()?;
    Some(Pdfium::new(bindings))
}

/// Rasterize page 1 (index 0) of `pdf_bytes` at 72 DPI (1pt = 1px for US-Letter → 612×792).
fn rasterize_page1(pdfium: &Pdfium, pdf_bytes: &[u8]) -> RgbaImage {
    let doc = pdfium
        .load_pdf_from_byte_slice(pdf_bytes, None)
        .expect("rasterize: load PDF");
    let page = doc.pages().get(0).expect("rasterize: get page 0");
    let config = PdfRenderConfig::new()
        .set_target_width(612)
        .set_target_height(792);
    let bitmap = page
        .render_with_config(&config)
        .expect("rasterize: render page");
    bitmap.as_image().into_rgba8()
}

// ── Pixel diff ────────────────────────────────────────────────────────────────

/// Returns the fraction of pixels that differ between `actual` and `expected`.
/// Returns 1.0 (100 %) when dimensions do not match.
fn pixel_diff_ratio(actual: &RgbaImage, expected: &RgbaImage) -> f64 {
    if actual.dimensions() != expected.dimensions() {
        return 1.0;
    }
    let total = (actual.width() * actual.height()) as f64;
    let differing = actual
        .pixels()
        .zip(expected.pixels())
        .filter(|(a, e)| a != e)
        .count() as f64;
    differing / total
}

/// Produce a diff image: changed pixels are highlighted red on a copy of `actual`.
fn make_diff_image(actual: &RgbaImage, expected: &RgbaImage) -> RgbaImage {
    let mut diff = actual.clone();
    let (w, h) = (
        actual.width().min(expected.width()),
        actual.height().min(expected.height()),
    );
    for y in 0..h {
        for x in 0..w {
            if actual.get_pixel(x, y) != expected.get_pixel(x, y) {
                diff.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
    }
    diff
}

// ── Golden comparison ─────────────────────────────────────────────────────────

fn assert_visual_golden(test_name: &str, actual: &RgbaImage, tolerance: f64) {
    let golden_path = goldens_dir().join(format!("{test_name}.golden.png"));
    let actual_path = goldens_dir().join(format!("{test_name}.actual.png"));
    let diff_path   = goldens_dir().join(format!("{test_name}.diff.png"));

    if std::env::var("UPDATE_GOLDENS").is_ok() {
        std::fs::create_dir_all(goldens_dir()).expect("create goldens dir");
        actual
            .save_with_format(&golden_path, ImageFormat::Png)
            .unwrap_or_else(|e| panic!("Failed to write golden {}: {e}", golden_path.display()));
        eprintln!("[GOLDEN] Updated: {}", golden_path.display());
        return;
    }

    let expected = match image::open(&golden_path) {
        Ok(img) => img.into_rgba8(),
        Err(_) => {
            panic!(
                "Golden file missing: {}\n\n\
                 Generate goldens with:\n\
                   UPDATE_GOLDENS=1 ./scripts/cargo-slot.sh test -p contract-tests -- visual_drift\n\n\
                 Then review every changed file:\n\
                   git diff tools/contract-tests/goldens/\n\n\
                 Commit the goldens once you have verified they are correct.",
                golden_path.display()
            );
        }
    };

    let ratio = pixel_diff_ratio(actual, &expected);

    if ratio > tolerance {
        // Write artifacts for CI upload and human review.
        let _ = std::fs::create_dir_all(goldens_dir());
        let _ = actual.save_with_format(&actual_path, ImageFormat::Png);
        let diff_img = make_diff_image(actual, &expected);
        let _ = diff_img.save_with_format(&diff_path, ImageFormat::Png);

        panic!(
            "VISUAL DRIFT EXCEEDED [{test_name}]\n\
             Diff ratio:  {:.3} % (actual) > {:.3} % (tolerance)\n\
             Golden:      {}\n\
             Actual PNG:  {}\n\
             Diff PNG:    {}\n\n\
             If the change is intentional, regenerate goldens:\n\
               UPDATE_GOLDENS=1 ./scripts/cargo-slot.sh test -p contract-tests -- {test_name}",
            ratio * 100.0,
            tolerance * 100.0,
            golden_path.display(),
            actual_path.display(),
            diff_path.display(),
        );
    }
}

// ── Fixture loading ───────────────────────────────────────────────────────────

fn load_fixture_annotation(fixture_name: &str) -> pdf_editor::domain::annotations::types::Annotation {
    let path = consumer_fixture_path(fixture_name);
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Cannot read fixture {}: {e}", path.display()));
    serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("Cannot parse fixture {}: {e}", path.display()))
}

// ── Core harness ──────────────────────────────────────────────────────────────

fn run_visual_drift(pdfium: &Pdfium, fixture_name: &str, ann_type: AnnotationType) {
    let annotation = load_fixture_annotation(fixture_name);
    let base_pdf = base_pdf_bytes();
    let annotated = render_annotations(&base_pdf, &[annotation])
        .unwrap_or_else(|e| panic!("render_annotations failed for {fixture_name}: {e}"));
    let actual = rasterize_page1(pdfium, &annotated);
    let tolerance = class_tolerance(ann_type);
    assert_visual_golden(fixture_name, &actual, tolerance);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

macro_rules! visual_drift_test {
    ($fn_name:ident, $fixture:expr, $ann_type:expr) => {
        #[test]
        fn $fn_name() {
            let pdfium = match try_create_pdfium() {
                Some(p) => p,
                None => {
                    eprintln!(
                        "[visual_drift] Skipping {}: PDFIUM_LIB_PATH not set",
                        stringify!($fn_name)
                    );
                    return;
                }
            };
            run_visual_drift(&pdfium, $fixture, $ann_type);
        }
    };
}

visual_drift_test!(visual_drift_freehand,  "FREEHAND",  AnnotationType::Freehand);
visual_drift_test!(visual_drift_signature, "SIGNATURE", AnnotationType::Signature);
visual_drift_test!(visual_drift_bubble,    "BUBBLE",    AnnotationType::Bubble);
visual_drift_test!(visual_drift_callout,   "CALLOUT",   AnnotationType::Callout);
visual_drift_test!(visual_drift_arrow,     "ARROW",     AnnotationType::Arrow);
visual_drift_test!(visual_drift_text,      "TEXT",      AnnotationType::Text);
visual_drift_test!(visual_drift_stamp,     "STAMP",     AnnotationType::Stamp);
visual_drift_test!(visual_drift_shape,     "SHAPE",     AnnotationType::Shape);
visual_drift_test!(visual_drift_highlight, "HIGHLIGHT", AnnotationType::Highlight);
visual_drift_test!(visual_drift_whiteout,  "WHITEOUT",  AnnotationType::Whiteout);
