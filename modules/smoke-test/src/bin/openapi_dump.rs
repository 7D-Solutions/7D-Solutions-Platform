use utoipa::OpenApi;

fn main() {
    let spec = smoke_test::http::ApiDoc::openapi();
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).expect("serialize OpenAPI")
    );
}
