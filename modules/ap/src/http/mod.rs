pub mod admin;
pub mod admin_types;
pub mod allocations;
pub mod bills;
pub mod payment_runs;
pub mod payment_terms;
pub mod purchase_orders;
pub mod reports;
pub mod tax_reports;
pub mod tenant;
pub mod vendors;

use axum::{extract::State, http::StatusCode, Json};
use health::{
    build_ready_response, db_check_with_pool, ready_response_to_axum, PoolMetrics, ReadyResponse,
};
use std::sync::Arc;
use std::time::Instant;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "AP Service",
        version = "2.1.0",
        description = "Accounts payable: bills, purchase orders, payment runs, vendor management, and AP aging.",
    ),
    paths(
        // Vendors
        vendors::create_vendor,
        vendors::list_vendors,
        vendors::get_vendor,
        vendors::update_vendor,
        vendors::deactivate_vendor,
        // Purchase Orders
        purchase_orders::create_po,
        purchase_orders::get_po,
        purchase_orders::list_pos,
        purchase_orders::update_po_lines,
        purchase_orders::approve_po,
        // Bills
        bills::create_bill,
        bills::get_bill,
        bills::list_bills,
        bills::match_bill,
        bills::approve_bill,
        bills::void_bill,
        bills::quote_bill_tax,
        // Allocations
        allocations::create_allocation,
        allocations::list_allocations,
        allocations::get_balance,
        // Payment Terms
        payment_terms::create_terms,
        payment_terms::get_terms,
        payment_terms::list_terms,
        payment_terms::update_terms,
        payment_terms::assign_terms,
        // Payment Runs
        payment_runs::create_run,
        payment_runs::get_run,
        payment_runs::execute_run,
        // Reports
        reports::aging_report,
        tax_reports::tax_report_summary,
        tax_reports::tax_report_export,
    ),
    components(schemas(
        // Vendors
        crate::domain::vendors::Vendor,
        crate::domain::vendors::CreateVendorRequest,
        crate::domain::vendors::UpdateVendorRequest,
        // Purchase Orders
        crate::domain::po::PurchaseOrder,
        crate::domain::po::PurchaseOrderWithLines,
        crate::domain::po::CreatePoRequest,
        crate::domain::po::UpdatePoLinesRequest,
        crate::domain::po::ApprovePoRequest,
        // Bills
        crate::domain::bills::VendorBill,
        crate::domain::bills::VendorBillWithLines,
        crate::domain::bills::CreateBillRequest,
        crate::domain::bills::ApproveBillRequest,
        crate::domain::bills::VoidBillRequest,
        // Match
        crate::domain::r#match::RunMatchRequest,
        crate::domain::r#match::MatchOutcome,
        // Allocations
        crate::domain::allocations::AllocationRecord,
        crate::domain::allocations::BillBalanceSummary,
        crate::domain::allocations::CreateAllocationRequest,
        // Payment Terms
        crate::domain::payment_terms::PaymentTerms,
        crate::domain::payment_terms::CreatePaymentTermsRequest,
        crate::domain::payment_terms::UpdatePaymentTermsRequest,
        crate::domain::payment_terms::AssignTermsRequest,
        // Payment Runs
        payment_runs::CreatePaymentRunBody,
        payment_runs::PaymentRunItem,
        payment_runs::PaymentRunResponse,
        payment_runs::ExecutionEntry,
        payment_runs::ExecuteRunResponse,
        // Reports
        crate::domain::reports::aging::AgingReport,
        crate::domain::reports::aging::CurrencyBucket,
        crate::domain::reports::aging::VendorBucket,
        // Tax
        crate::domain::tax::ApTaxSnapshot,
        tax_reports::ApTaxReportResponse,
        crate::domain::tax::reports::ApTaxSummaryRow,
        // Platform
        platform_http_contracts::ApiError,
        platform_http_contracts::PaginatedResponse<crate::domain::vendors::Vendor>,
        platform_http_contracts::PaginatedResponse<crate::domain::bills::VendorBill>,
        platform_http_contracts::PaginatedResponse<crate::domain::po::PurchaseOrder>,
        platform_http_contracts::PaginatedResponse<crate::domain::payment_terms::PaymentTerms>,
        platform_http_contracts::PaginatedResponse<crate::domain::allocations::AllocationRecord>,
    )),
    security(("bearer" = [])),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

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

/// GET /api/health — liveness probe (legacy, kept for compat)
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "ap",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// GET /api/ready — readiness probe (verifies DB connectivity)
pub async fn ready(
    State(state): State<Arc<crate::AppState>>,
) -> Result<Json<ReadyResponse>, (StatusCode, Json<ReadyResponse>)> {
    let start = Instant::now();
    let db_err = sqlx::query("SELECT 1")
        .execute(&state.pool)
        .await
        .err()
        .map(|e| e.to_string());
    let latency = start.elapsed().as_millis() as u64;

    let pool_metrics = PoolMetrics {
        size: state.pool.size(),
        idle: state.pool.num_idle() as u32,
        active: state
            .pool
            .size()
            .saturating_sub(state.pool.num_idle() as u32),
    };

    let resp = build_ready_response(
        "ap",
        env!("CARGO_PKG_VERSION"),
        vec![db_check_with_pool(latency, db_err, pool_metrics)],
    );
    ready_response_to_axum(resp)
}

/// GET /api/version — module identity and schema version
pub async fn version() -> Json<serde_json::Value> {
    const SCHEMA_VERSION: &str = "20260218000001";

    Json(serde_json::json!({
        "module_name": "ap",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}

/// GET /api/schema-version — schema version only
pub async fn schema_version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "schema_version": "20260218000001"
    }))
}
