//! JSON-schema endpoint for annotation payloads (bd-rxvbr).
//!
//! Route: GET /api/schemas/annotations/v{version}
//!
//! Returns JSON-schema for the annotation payload at the requested version.
//! Responds with 24h Cache-Control so CDNs and clients cache aggressively.
//! Returns 404 for versions outside the supported range.

use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use crate::domain::annotations::types::CURRENT_ANNOTATION_SCHEMA_VERSION;

const MIN_SCHEMA_VERSION: u32 = 1;

pub async fn annotation_schema(Path(version): Path<u32>) -> impl IntoResponse {
    if version < MIN_SCHEMA_VERSION || version > CURRENT_ANNOTATION_SCHEMA_VERSION {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            Json(json!({
                "error": format!(
                    "schema version {} not found; supported: {}-{}",
                    version, MIN_SCHEMA_VERSION, CURRENT_ANNOTATION_SCHEMA_VERSION
                )
            })),
        )
            .into_response();
    }

    let schema = build_schema_v1();
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/schema+json"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        Json(schema),
    )
        .into_response()
}

fn build_schema_v1() -> Value {
    let mut properties = serde_json::Map::new();

    // Core fields
    properties.insert("schemaVersion".into(), json!({"type":"integer","description":"Schema version. Defaults to 1 when absent.","default":1,"minimum":1,"maximum":1}));
    properties.insert("id".into(), json!({"type":"string"}));
    properties.insert("x".into(), json!({"type":"number"}));
    properties.insert("y".into(), json!({"type":"number"}));
    properties.insert("pageNumber".into(), json!({"type":"integer","minimum":1}));
    properties.insert("type".into(), json!({
        "type":"string",
        "enum":["CALLOUT","ARROW","HIGHLIGHT","STAMP","SHAPE","FREEHAND","TEXT","BUBBLE","SIGNATURE","WHITEOUT"]
    }));

    // Text properties
    properties.insert("text".into(), json!({"type":["string","null"]}));
    properties.insert("fontSize".into(), json!({"type":["number","null"]}));
    properties.insert("fontFamily".into(), json!({"type":["string","null"]}));
    properties.insert("fontWeight".into(), json!({"type":["string","null"]}));
    properties.insert("fontStyle".into(), json!({"type":["string","null"]}));

    // Colors
    properties.insert("color".into(), json!({"type":["string","null"]}));
    properties.insert("bgColor".into(), json!({"type":["string","null"]}));
    properties.insert("borderColor".into(), json!({"type":["string","null"]}));

    // Arrow
    properties.insert("x2".into(), json!({"type":["number","null"]}));
    properties.insert("y2".into(), json!({"type":["number","null"]}));
    properties.insert("arrowheadSize".into(), json!({"type":["number","null"]}));
    properties.insert("strokeWidth".into(), json!({"type":["number","null"]}));

    // Shape
    properties.insert("shapeType".into(), json!({"type":["string","null"],"enum":["RECTANGLE","CIRCLE","LINE","POLYGON","REVISION_CLOUD",null]}));
    properties.insert("width".into(), json!({"type":["number","null"]}));
    properties.insert("height".into(), json!({"type":["number","null"]}));

    // Highlight
    properties.insert("opacity".into(), json!({"type":["number","null"]}));

    // Stamp
    properties.insert("stampType".into(), json!({"type":["string","null"],"enum":["APPROVED","REJECTED","FAI_REQUIRED","HOLD","REVIEWED","VERIFIED","CUSTOM",null]}));
    properties.insert("stampUsername".into(), json!({"type":["string","null"]}));
    properties.insert("stampDate".into(), json!({"type":["string","null"]}));
    properties.insert("stampTime".into(), json!({"type":["string","null"]}));

    // Bubble
    properties.insert("bubbleNumber".into(), json!({"type":["integer","null"]}));
    properties.insert("bubbleSize".into(), json!({"type":["number","null"]}));
    properties.insert("bubbleColor".into(), json!({"type":["string","null"]}));
    properties.insert("bubbleBorderColor".into(), json!({"type":["string","null"]}));
    properties.insert("textColor".into(), json!({"type":["string","null"]}));
    properties.insert("bubbleFontSize".into(), json!({"type":["number","null"]}));
    properties.insert("bubbleShape".into(), json!({"type":["string","null"],"enum":["CIRCLE","SQUARE","OVAL",null]}));
    properties.insert("hasLeaderLine".into(), json!({"type":["boolean","null"]}));
    properties.insert("leaderX".into(), json!({"type":["number","null"]}));
    properties.insert("leaderY".into(), json!({"type":["number","null"]}));
    properties.insert("leaderColor".into(), json!({"type":["string","null"]}));
    properties.insert("leaderStrokeWidth".into(), json!({"type":["number","null"]}));

    // Signature
    properties.insert("signatureMethod".into(), json!({"type":["string","null"]}));
    properties.insert("signatureImage".into(), json!({"type":["string","null"]}));
    properties.insert("signatureText".into(), json!({"type":["string","null"]}));

    Value::Object({
        let mut root = serde_json::Map::new();
        root.insert("$schema".into(), json!("https://json-schema.org/draft/2020-12/schema"));
        root.insert("$id".into(), json!("https://7dsolutions.app/schemas/annotations/v1"));
        root.insert("title".into(), json!("Annotation"));
        root.insert("description".into(), json!("A single annotation on a PDF page (schema_version=1)."));
        root.insert("type".into(), json!("object"));
        root.insert("required".into(), json!(["id","x","y","pageNumber","type"]));
        root.insert("properties".into(), Value::Object(properties));
        root
    })
}
