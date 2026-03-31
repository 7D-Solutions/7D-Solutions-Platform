use utoipa::OpenApi;

fn main() {
    let spec = workflow::http::ApiDoc::openapi();
    let json = serde_json::to_string_pretty(&spec).expect("Failed to serialize OpenAPI spec");
    println!("{}", json);
}
