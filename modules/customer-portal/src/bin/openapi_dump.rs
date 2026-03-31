use utoipa::OpenApi;

fn main() {
    let spec = customer_portal::http::ApiDoc::openapi();
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).expect("serialize OpenAPI")
    );
}
