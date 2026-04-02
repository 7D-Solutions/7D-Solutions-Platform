//! Utility binary that prints the Identity & Auth OpenAPI spec as JSON to stdout.
//! No database or NATS connection required -- the spec is generated at compile time.
//!
//! Usage:  cargo run -p auth-rs --bin openapi_dump > openapi.json

use utoipa::OpenApi;

fn main() {
    let spec = auth_rs::routes::ApiDoc::openapi();
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).expect("serialize OpenAPI")
    );
}
