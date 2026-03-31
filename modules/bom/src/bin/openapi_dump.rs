use utoipa::OpenApi;

use bom_rs::domain::{
    eco_models::{
        ApplyEcoRequest, CreateEcoRequest, Eco, EcoActionRequest, EcoAuditEntry, EcoBomRevision,
        EcoDocRevision, LinkBomRevisionRequest, LinkDocRevisionRequest,
    },
    models::{
        AddLineRequest, BomHeader, BomLine, BomRevision, CreateBomRequest, CreateRevisionRequest,
        ExplosionRow, SetEffectivityRequest, UpdateLineRequest, WhereUsedRow,
    },
};
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "BOM Service",
        version = "2.2.0",
        description = "Bill of Materials: multi-level BOM structure with revisions, effectivity, \
                        explosion, and where-used queries. Engineering Change Orders (ECO) with \
                        full lifecycle (draft → submit → approve/reject → apply).",
    ),
    paths(
        bom_rs::http::bom_routes::list_boms,
        bom_rs::http::bom_routes::post_bom,
        bom_rs::http::bom_routes::get_bom,
        bom_rs::http::bom_routes::get_bom_by_part_id,
        bom_rs::http::bom_routes::post_revision,
        bom_rs::http::bom_routes::list_revisions,
        bom_rs::http::bom_routes::post_effectivity,
        bom_rs::http::bom_routes::post_line,
        bom_rs::http::bom_routes::get_lines,
        bom_rs::http::bom_routes::put_line,
        bom_rs::http::bom_routes::delete_line,
        bom_rs::http::bom_routes::get_explosion,
        bom_rs::http::bom_routes::get_where_used,
        bom_rs::http::eco_routes::post_eco,
        bom_rs::http::eco_routes::get_eco,
        bom_rs::http::eco_routes::get_eco_history_for_part,
        bom_rs::http::eco_routes::get_eco_audit,
        bom_rs::http::eco_routes::post_submit,
        bom_rs::http::eco_routes::post_approve,
        bom_rs::http::eco_routes::post_reject,
        bom_rs::http::eco_routes::post_apply,
        bom_rs::http::eco_routes::post_link_bom_revision,
        bom_rs::http::eco_routes::get_bom_revision_links,
        bom_rs::http::eco_routes::post_link_doc_revision,
        bom_rs::http::eco_routes::get_doc_revision_links,
    ),
    components(schemas(
        BomHeader, BomRevision, BomLine, ExplosionRow, WhereUsedRow,
        CreateBomRequest, CreateRevisionRequest, SetEffectivityRequest,
        AddLineRequest, UpdateLineRequest,
        Eco, EcoAuditEntry, EcoBomRevision, EcoDocRevision,
        CreateEcoRequest, LinkBomRevisionRequest, LinkDocRevisionRequest,
        EcoActionRequest, ApplyEcoRequest,
        ApiError, PaginatedResponse<BomHeader>, PaginatedResponse<BomRevision>,
        PaginatedResponse<BomLine>, PaginatedResponse<Eco>,
        PaginatedResponse<EcoAuditEntry>, PaginatedResponse<EcoBomRevision>,
        PaginatedResponse<EcoDocRevision>, PaginationMeta,
    )),
    security(("bearer" = [])),
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

fn main() {
    let spec = ApiDoc::openapi();
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).expect("serialize OpenAPI")
    );
}
