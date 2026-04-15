use axum::{routing::get, Json};
use std::sync::Arc;
use utoipa::OpenApi;

use party_rs::domain::address::{Address, CreateAddressRequest, UpdateAddressRequest};
use party_rs::domain::contact::{
    Contact, CreateContactRequest, PrimaryContactEntry, SetPrimaryRequest, UpdateContactRequest,
};
use party_rs::domain::party::{
    CreateCompanyRequest, CreateIndividualRequest, ExternalRef, Party, PartyCompany,
    PartyIndividual, PartyView, SearchQuery, UpdatePartyRequest,
};
use party_rs::http::party::ListPartiesQuery;
use party_rs::{http, metrics, AppState};
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};
use platform_sdk::ModuleBuilder;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Party Service",
        version = "2.1.0",
        description = "Party master data: companies, individuals, contacts, and addresses.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims \
                        (not headers). Permission: `party.mutate` for writes, reads are open to \
                        any authenticated caller.\n\n\
                        **Multi-tenancy:** All data is scoped by the `app_id` (tenant) in the JWT. \
                        No X-Tenant-Id header required.",
    ),
    paths(
        // Parties
        party_rs::http::party::create_company,
        party_rs::http::party::create_individual,
        party_rs::http::party::list_parties,
        party_rs::http::party::get_party,
        party_rs::http::party::update_party,
        party_rs::http::party::deactivate_party,
        party_rs::http::party::reactivate_party,
        party_rs::http::party::search_parties,
        // Contacts
        party_rs::http::contacts::create_contact,
        party_rs::http::contacts::list_contacts,
        party_rs::http::contacts::get_contact,
        party_rs::http::contacts::update_contact,
        party_rs::http::contacts::delete_contact,
        party_rs::http::contacts::set_primary,
        party_rs::http::contacts::primary_contacts,
        // Addresses
        party_rs::http::addresses::create_address,
        party_rs::http::addresses::list_addresses,
        party_rs::http::addresses::get_address,
        party_rs::http::addresses::update_address,
        party_rs::http::addresses::delete_address,
    ),
    components(schemas(
        // Party types
        Party, PartyCompany, PartyIndividual, ExternalRef, PartyView,
        CreateCompanyRequest, CreateIndividualRequest, UpdatePartyRequest,
        ListPartiesQuery, SearchQuery,
        // Contact types
        Contact, CreateContactRequest, UpdateContactRequest,
        SetPrimaryRequest, PrimaryContactEntry,
        // Address types
        Address, CreateAddressRequest, UpdateAddressRequest,
        // Shared envelopes
        ApiError, PaginatedResponse<Party>, PaginatedResponse<PrimaryContactEntry>, PaginationMeta,
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
            let party_metrics =
                Arc::new(metrics::PartyMetrics::new().expect("Party: failed to create metrics"));
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: party_metrics,
            });
            http::router(app_state).route("/api/openapi.json", get(openapi_json))
        })
        .run()
        .await
        .expect("party module failed");
}
