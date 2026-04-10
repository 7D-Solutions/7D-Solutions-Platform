//! Utility binary that prints the Party OpenAPI spec as JSON to stdout.
//! No database or NATS connection required — the spec is generated at compile time.
//!
//! Usage:  cargo run --bin openapi_dump > openapi.json

use serde_json::to_string_pretty;
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
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Party Service",
        version = "2.2.0",
        description = "Party master data: companies, individuals, contacts, and addresses.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims \
                        (not headers). Permission: `party.mutate` for writes, reads are open to \
                        any authenticated caller.\n\n\
                        **Multi-tenancy:** All data is scoped by the `app_id` (tenant) in the JWT. \
                        No X-Tenant-Id header required.",
    ),
    paths(
        party_rs::http::party::create_company,
        party_rs::http::party::create_individual,
        party_rs::http::party::list_parties,
        party_rs::http::party::get_party,
        party_rs::http::party::update_party,
        party_rs::http::party::deactivate_party,
        party_rs::http::party::reactivate_party,
        party_rs::http::party::search_parties,
        party_rs::http::contacts::create_contact,
        party_rs::http::contacts::list_contacts,
        party_rs::http::contacts::get_contact,
        party_rs::http::contacts::update_contact,
        party_rs::http::contacts::delete_contact,
        party_rs::http::contacts::set_primary,
        party_rs::http::contacts::primary_contacts,
        party_rs::http::addresses::create_address,
        party_rs::http::addresses::list_addresses,
        party_rs::http::addresses::get_address,
        party_rs::http::addresses::update_address,
        party_rs::http::addresses::delete_address,
    ),
    components(schemas(
        Party, PartyCompany, PartyIndividual, ExternalRef, PartyView,
        CreateCompanyRequest, CreateIndividualRequest, UpdatePartyRequest,
        ListPartiesQuery, SearchQuery,
        Contact, CreateContactRequest, UpdateContactRequest,
        SetPrimaryRequest, PrimaryContactEntry,
        Address, CreateAddressRequest, UpdateAddressRequest,
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

fn main() {
    let spec = ApiDoc::openapi();
    println!("{}", to_string_pretty(&spec).expect("serialize OpenAPI"));
}
