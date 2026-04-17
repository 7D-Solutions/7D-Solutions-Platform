use axum::{routing::get, Json, Router};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;
use utoipa::OpenApi;

use bom_rs::{
    domain::{
        eco_models::{
            ApplyEcoRequest, CreateEcoRequest, Eco, EcoActionRequest, EcoAuditEntry,
            EcoBomRevision, EcoDocRevision, LinkBomRevisionRequest, LinkDocRevisionRequest,
        },
        models::{
            AddLineRequest, BomHeader, BomLine, BomLineEnriched, BomRevision, CreateBomRequest,
            CreateRevisionRequest, ExplosionRow, ItemDetails, MrpExplodeRequest, MrpRequirementLine,
            MrpSnapshot, MrpSnapshotWithLines, OnHandEntry,
            SetEffectivityRequest, UpdateLineRequest, WhereUsedRow,
        },
    },
    http::{
        bom_routes::{
            delete_line, get_bom, get_bom_by_part_id, get_explosion, get_lines, get_revision,
            get_where_used, list_boms, list_revisions, post_bom, post_effectivity, post_line,
            post_revision, put_line,
        },
        eco_routes::{
            get_bom_revision_links, get_doc_revision_links, get_eco, get_eco_audit,
            get_eco_history_for_part, post_apply, post_approve, post_eco, post_link_bom_revision,
            post_link_doc_revision, post_reject, post_submit,
        },
        mrp_routes::{get_mrp_snapshot, list_mrp_snapshots, post_mrp_explode},
    },
    metrics::BomMetrics,
    AppState, InventoryClient, NumberingClient,
};
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};
use platform_sdk::ModuleBuilder;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "BOM Service",
        version = "2.2.0",
        description = "Bill of Materials: multi-level BOM structure with revisions, effectivity, \
                        explosion, and where-used queries. Engineering Change Orders (ECO) with \
                        full lifecycle (draft → submit → approve/reject → apply).\n\n\
                        **Service dependency:** Requires `NUMBERING_URL` (the Numbering service) \
                        for automatic ECO number allocation.",
    ),
    paths(
        // Health
        bom_rs::http::health::health,
        bom_rs::http::health::ready,
        bom_rs::http::health::version,
        // BOM Header
        bom_rs::http::bom_routes::list_boms,
        bom_rs::http::bom_routes::post_bom,
        bom_rs::http::bom_routes::get_bom,
        bom_rs::http::bom_routes::get_bom_by_part_id,
        // BOM Revisions
        bom_rs::http::bom_routes::post_revision,
        bom_rs::http::bom_routes::list_revisions,
        bom_rs::http::bom_routes::get_revision,
        bom_rs::http::bom_routes::post_effectivity,
        // BOM Lines
        bom_rs::http::bom_routes::post_line,
        bom_rs::http::bom_routes::get_lines,
        bom_rs::http::bom_routes::put_line,
        bom_rs::http::bom_routes::delete_line,
        // Explosion + Where-Used
        bom_rs::http::bom_routes::get_explosion,
        bom_rs::http::bom_routes::get_where_used,

        // MRP
        bom_rs::http::mrp_routes::post_mrp_explode,
        bom_rs::http::mrp_routes::get_mrp_snapshot,
        bom_rs::http::mrp_routes::list_mrp_snapshots,

        // ECO
        bom_rs::http::eco_routes::post_eco,
        bom_rs::http::eco_routes::get_eco,
        bom_rs::http::eco_routes::get_eco_history_for_part,
        bom_rs::http::eco_routes::get_eco_audit,
        // ECO Lifecycle
        bom_rs::http::eco_routes::post_submit,
        bom_rs::http::eco_routes::post_approve,
        bom_rs::http::eco_routes::post_reject,
        bom_rs::http::eco_routes::post_apply,
        // ECO Links
        bom_rs::http::eco_routes::post_link_bom_revision,
        bom_rs::http::eco_routes::get_bom_revision_links,
        bom_rs::http::eco_routes::post_link_doc_revision,
        bom_rs::http::eco_routes::get_doc_revision_links,
    ),
    components(schemas(
        BomHeader, BomRevision, BomLine, BomLineEnriched, ItemDetails, ExplosionRow, WhereUsedRow,
        CreateBomRequest, CreateRevisionRequest, SetEffectivityRequest,
        AddLineRequest, UpdateLineRequest,
        MrpSnapshot, MrpRequirementLine, MrpSnapshotWithLines, OnHandEntry, MrpExplodeRequest,
        Eco, EcoAuditEntry, EcoBomRevision, EcoDocRevision,
        CreateEcoRequest, LinkBomRevisionRequest, LinkDocRevisionRequest,
        EcoActionRequest, ApplyEcoRequest,
        ApiError, PaginatedResponse<BomHeader>, PaginatedResponse<BomRevision>,
        PaginatedResponse<BomLine>, PaginatedResponse<Eco>,
        PaginatedResponse<EcoAuditEntry>, PaginatedResponse<EcoBomRevision>,
        PaginatedResponse<EcoDocRevision>, PaginatedResponse<MrpSnapshot>, PaginationMeta,
    )),
    security(
        ("bearer" = [])
    ),
    modifiers(&SecurityAddon),
)]
struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::HttpBuilder::new()
                    .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let metrics = Arc::new(BomMetrics::new().expect("Failed to create metrics registry"));
            let numbering = ctx.platform_client::<NumberingClient>();
            let inventory = ctx.platform_client::<InventoryClient>();
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics,
                numbering,
                inventory,
            });

            let mrp_mutations = Router::new()
                .route("/api/bom/mrp/explode", axum::routing::post(post_mrp_explode))
                .route_layer(RequirePermissionsLayer::new(&[permissions::BOM_MUTATE]))
                .with_state(app_state.clone());

            let mrp_reads = Router::new()
                .route("/api/bom/mrp/snapshots", axum::routing::get(list_mrp_snapshots))
                .route(
                    "/api/bom/mrp/snapshots/{snapshot_id}",
                    axum::routing::get(get_mrp_snapshot),
                )
                .route_layer(RequirePermissionsLayer::new(&[permissions::BOM_READ]))
                .with_state(app_state.clone());

            let bom_mutations = Router::new()
                .route("/api/bom", axum::routing::post(post_bom))
                .route(
                    "/api/bom/{bom_id}/revisions",
                    axum::routing::post(post_revision),
                )
                .route(
                    "/api/bom/revisions/{revision_id}/effectivity",
                    axum::routing::post(post_effectivity),
                )
                .route(
                    "/api/bom/revisions/{revision_id}/lines",
                    axum::routing::post(post_line),
                )
                .route(
                    "/api/bom/lines/{line_id}",
                    axum::routing::put(put_line).delete(delete_line),
                )
                // ECO mutations
                .route("/api/eco", axum::routing::post(post_eco))
                .route("/api/eco/{eco_id}/submit", axum::routing::post(post_submit))
                .route(
                    "/api/eco/{eco_id}/approve",
                    axum::routing::post(post_approve),
                )
                .route("/api/eco/{eco_id}/reject", axum::routing::post(post_reject))
                .route("/api/eco/{eco_id}/apply", axum::routing::post(post_apply))
                .route(
                    "/api/eco/{eco_id}/bom-revisions",
                    axum::routing::post(post_link_bom_revision),
                )
                .route(
                    "/api/eco/{eco_id}/doc-revisions",
                    axum::routing::post(post_link_doc_revision),
                )
                .route_layer(RequirePermissionsLayer::new(&[permissions::BOM_MUTATE]))
                .with_state(app_state.clone());

            let bom_reads = Router::new()
                .route("/api/bom", axum::routing::get(list_boms))
                .route("/api/bom/{bom_id}", axum::routing::get(get_bom))
                .route(
                    "/api/bom/by-part/{part_id}",
                    axum::routing::get(get_bom_by_part_id),
                )
                .route(
                    "/api/bom/{bom_id}/revisions",
                    axum::routing::get(list_revisions),
                )
                .route(
                    "/api/bom/revisions/{revision_id}",
                    axum::routing::get(get_revision),
                )
                .route(
                    "/api/bom/revisions/{revision_id}/lines",
                    axum::routing::get(get_lines),
                )
                .route(
                    "/api/bom/{bom_id}/explosion",
                    axum::routing::get(get_explosion),
                )
                .route(
                    "/api/bom/where-used/{item_id}",
                    axum::routing::get(get_where_used),
                )
                // ECO reads
                .route("/api/eco/{eco_id}", axum::routing::get(get_eco))
                .route(
                    "/api/eco/{eco_id}/bom-revisions",
                    axum::routing::get(get_bom_revision_links),
                )
                .route(
                    "/api/eco/{eco_id}/doc-revisions",
                    axum::routing::get(get_doc_revision_links),
                )
                .route("/api/eco/{eco_id}/audit", axum::routing::get(get_eco_audit))
                .route(
                    "/api/eco/history/{part_id}",
                    axum::routing::get(get_eco_history_for_part),
                )
                .route_layer(RequirePermissionsLayer::new(&[permissions::BOM_READ]))
                .with_state(app_state);

            Router::new()
                .merge(bom_reads)
                .merge(bom_mutations)
                .merge(mrp_reads)
                .merge(mrp_mutations)
                .route("/api/openapi.json", get(openapi_json))
        })
        .run()
        .await
        .expect("bom module failed");
}
