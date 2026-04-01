//! Utility binary that prints the Fixed Assets OpenAPI spec as JSON to stdout.
//! No database or NATS connection required — the spec is generated at compile time.
//!
//! Usage:  cargo run --bin openapi_dump > openapi.json

use utoipa::OpenApi;

fn main() {
    let spec = fixed_assets::http::ApiDoc::openapi();
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).expect("serialize OpenAPI")
    );
}
