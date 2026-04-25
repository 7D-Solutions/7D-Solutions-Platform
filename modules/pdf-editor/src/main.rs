// pdf-editor v2.3.0
use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post, put},
    Json, Router,
};
use utoipa::OpenApi;

use pdf_editor::http as handlers;
use platform_sdk::ModuleBuilder;
use security::{permissions, RequirePermissionsLayer};

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(handlers::ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    pdf_editor::domain::annotations::render::assert_pdfium_abi();

    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let db = ctx.pool().clone();

            let mutations = Router::new()
                .route(
                    "/api/pdf/forms/templates",
                    post(handlers::templates::create_template),
                )
                .route(
                    "/api/pdf/forms/templates/{id}",
                    put(handlers::templates::update_template),
                )
                .route(
                    "/api/pdf/forms/templates/{id}/fields",
                    post(handlers::fields::create_field),
                )
                .route(
                    "/api/pdf/forms/templates/{tid}/fields/{fid}",
                    put(handlers::fields::update_field),
                )
                .route(
                    "/api/pdf/forms/templates/{id}/fields/reorder",
                    post(handlers::fields::reorder_fields),
                )
                .route(
                    "/api/pdf/forms/submissions",
                    post(handlers::submissions::create_submission),
                )
                .route(
                    "/api/pdf/forms/submissions/{id}",
                    put(handlers::submissions::autosave_submission),
                )
                .route(
                    "/api/pdf/forms/submissions/{id}/submit",
                    post(handlers::submissions::submit_submission),
                )
                .route(
                    "/api/pdf/forms/submissions/{id}/generate",
                    post(handlers::generate::generate_pdf),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::PDF_EDITOR_MUTATE,
                ]))
                .with_state(db.clone());

            let reads = Router::new()
                .route(
                    "/api/pdf/forms/templates",
                    get(handlers::templates::list_templates),
                )
                .route(
                    "/api/pdf/forms/templates/{id}",
                    get(handlers::templates::get_template),
                )
                .route(
                    "/api/pdf/forms/templates/{id}/fields",
                    get(handlers::fields::list_fields),
                )
                .route(
                    "/api/pdf/forms/submissions",
                    get(handlers::submissions::list_submissions),
                )
                .route(
                    "/api/pdf/forms/submissions/{id}",
                    get(handlers::submissions::get_submission),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::PDF_EDITOR_READ,
                ]))
                .with_state(db);

            Router::new()
                .merge(mutations)
                .merge(reads)
                .merge(
                    Router::new()
                        .route(
                            "/api/pdf/render-annotations",
                            post(handlers::annotations::render_annotations),
                        )
                        .route_layer(RequirePermissionsLayer::new(&[
                            permissions::PDF_EDITOR_MUTATE,
                        ]))
                        .layer(DefaultBodyLimit::max(52_428_800)),
                )
                .route("/api/openapi.json", get(openapi_json))
                .route(
                    "/api/schemas/annotations/v{version}",
                    get(handlers::schemas::annotation_schema),
                )
        })
        .run()
        .await
        .expect("pdf-editor module failed");
}
