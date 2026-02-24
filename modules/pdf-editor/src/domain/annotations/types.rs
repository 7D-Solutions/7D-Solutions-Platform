use serde::Deserialize;

/// Annotation types matching the frontend's AnnotationType union.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnnotationType {
    Callout,
    Arrow,
    Highlight,
    Stamp,
    Shape,
    Freehand,
    Text,
    Bubble,
    Signature,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StampType {
    Approved,
    Rejected,
    FaiRequired,
    Hold,
    Reviewed,
    Verified,
    Custom,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ShapeType {
    Rectangle,
    Circle,
    Line,
    Polygon,
    RevisionCloud,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TextRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignaturePoint {
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub new_stroke: Option<bool>,
}

/// An annotation from the frontend, matching the TypeScript Annotation type.
///
/// All coordinates are in PDF user-space units (points) relative to the page.
/// The page_number field is 1-based.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub page_number: u32,
    #[serde(rename = "type")]
    pub annotation_type: AnnotationType,

    // Text properties
    pub text: Option<String>,
    pub font_size: Option<f32>,
    #[serde(default = "default_font_family")]
    pub font_family: Option<String>,
    pub font_weight: Option<String>,
    pub font_style: Option<String>,

    // Colors
    pub color: Option<String>,
    pub bg_color: Option<String>,
    pub border_color: Option<String>,

    // Arrow-specific
    pub x2: Option<f32>,
    pub y2: Option<f32>,
    pub arrowhead_size: Option<f32>,
    pub stroke_width: Option<f32>,

    // Shape-specific
    pub shape_type: Option<ShapeType>,
    pub width: Option<f32>,
    pub height: Option<f32>,

    // Highlight-specific
    pub opacity: Option<f32>,
    pub text_rects: Option<Vec<TextRect>>,

    // Stamp-specific
    pub stamp_type: Option<StampType>,
    pub stamp_username: Option<String>,
    pub stamp_date: Option<String>,
    pub stamp_time: Option<String>,

    // Freehand-specific
    pub path: Option<Vec<Point>>,

    // Bubble-specific
    pub bubble_number: Option<u32>,
    pub bubble_size: Option<f32>,
    pub bubble_color: Option<String>,
    pub bubble_border_color: Option<String>,
    pub text_color: Option<String>,
    pub bubble_font_size: Option<f32>,
    pub has_leader_line: Option<bool>,
    pub leader_x: Option<f32>,
    pub leader_y: Option<f32>,
    pub leader_color: Option<String>,
    pub leader_stroke_width: Option<f32>,

    // Signature-specific
    pub signature_method: Option<String>,
    pub signature_path: Option<Vec<SignaturePoint>>,
    pub signature_image: Option<String>,
    pub signature_text: Option<String>,
}

fn default_font_family() -> Option<String> {
    None
}
