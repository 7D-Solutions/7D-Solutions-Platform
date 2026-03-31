use utoipa::OpenApi;

fn main() {
    let spec = workforce_competence_rs::http::ApiDoc::openapi();
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).expect("serialize OpenAPI")
    );
}
