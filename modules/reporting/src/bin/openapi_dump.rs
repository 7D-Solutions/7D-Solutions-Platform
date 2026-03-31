//! Utility binary that prints the Reporting OpenAPI spec as JSON to stdout.
//! No database or NATS connection required — the spec is generated at compile time.
//!
//! Usage:  cargo run --bin openapi_dump > openapi.json

use utoipa::OpenApi;

use platform_http_contracts::ApiError;
use reporting::domain::{
    aging::{ap_aging, ar_aging},
    forecast::types::{AtRiskItem, CashForecastResponse, CurrencyForecast, ForecastHorizon},
    jobs::snapshot_runner::SnapshotRunResult,
    kpis::KpiSnapshot,
    statements::{
        balance_sheet::{BalanceSheet, BsAccountLine, BsSection},
        cashflow::{CashflowLine, CashflowSection, CashflowStatement},
        pl::{PlAccountLine, PlSection, PlStatement},
    },
};
use reporting::http::{
    admin::RebuildRequest,
    aging::ArAgingResponse,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Reporting Service",
        version = "2.1.0",
        description = "Financial reporting: aging, KPIs, statements, cash flow forecasts, \
                        and probabilistic collection forecasting.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims. \
                        Permissions: `REPORTING_READ` for queries, `REPORTING_MUTATE` for writes.",
    ),
    paths(
        reporting::http::statements::get_pl,
        reporting::http::statements::get_balance_sheet,
        reporting::http::cashflow::get_cashflow,
        reporting::http::aging::get_ar_aging,
        reporting::http::aging::get_ap_aging,
        reporting::http::kpis::get_kpis,
        reporting::http::forecast::get_forecast,
        reporting::http::admin::rebuild,
    ),
    components(schemas(
        PlStatement, PlSection, PlAccountLine,
        BalanceSheet, BsSection, BsAccountLine,
        CashflowStatement, CashflowSection, CashflowLine,
        ar_aging::ArAgingSummary, ar_aging::ArAgingRow,
        ArAgingResponse,
        ap_aging::ApAgingReport, ap_aging::VendorAgingRow, ap_aging::CurrencySummary,
        KpiSnapshot,
        CashForecastResponse, CurrencyForecast, ForecastHorizon, AtRiskItem,
        SnapshotRunResult,
        RebuildRequest,
        ApiError,
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
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).expect("serialize OpenAPI")
    );
}
