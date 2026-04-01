pub mod admin;
pub mod aging;
pub mod allocation;
pub mod charges;
pub mod credit_notes;
pub mod customers;
pub mod disputes;
pub mod dunning_routes;
pub mod events;
pub mod health;
pub mod invoices;
pub mod payment_methods;
pub mod reconciliation_routes;
pub mod refunds;
pub mod subscriptions;
pub mod tax;
pub mod tax_config;
pub mod tax_config_rules;
pub mod tax_reports;
pub mod tenant;
pub mod usage;
pub mod webhooks;
pub mod write_offs;

use axum::{
    middleware,
    routing::{get, post, put},
    Router,
};
use security::{permissions, ratelimit::WebhookRateLimiter, RequirePermissionsLayer};
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::OpenApi;

use crate::idempotency::check_idempotency;
use crate::middleware::{webhook_ratelimit_middleware, WebhookRateLimitState};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "AR Service",
        version = "2.3.1",
        description = "Invoicing, collections, payment application, dunning, and cash flow forecasting.\n\n\
                        **Authentication:** Bearer JWT. Tenant derived from JWT claims.\n\
                        Permissions: `ar.read` for queries, `ar.mutate` for writes."
    ),
    components(schemas(
        platform_http_contracts::ApiError,
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

/// Build the AR router with full permission enforcement (production).
pub fn ar_router(db: PgPool) -> Router {
    build_ar_router(db, true)
}

/// Build the AR router without permission enforcement — integration tests only.
///
/// Bypasses the `ar.read` and `ar.mutate` permission gates so routes can be
/// exercised without JWT infrastructure. Do NOT use in production code.
pub fn ar_router_permissive(db: PgPool) -> Router {
    build_ar_router(db, false)
}

fn build_ar_router(db: PgPool, enforce_permissions: bool) -> Router {
    // Shared IP-based rate limiter for inbound webhook endpoints.
    let webhook_rl_state = Arc::new(WebhookRateLimitState {
        limiter: Arc::new(WebhookRateLimiter::new()),
    });

    // Inbound webhook sub-router — rate-limited by source IP (no auth needed).
    let webhook_inbound = Router::new()
        .route(
            "/api/ar/webhooks/tilled",
            post(webhooks::receive_tilled_webhook),
        )
        .layer(middleware::from_fn_with_state(
            webhook_rl_state,
            webhook_ratelimit_middleware,
        ))
        .with_state(db.clone());

    // Mutation routes — in production require ar.mutate permission.
    let mutations_core = Router::new()
        // Customers — write
        .route("/api/ar/customers", post(customers::create_customer))
        .route("/api/ar/customers/{id}", put(customers::update_customer))
        // Subscriptions — write
        .route(
            "/api/ar/subscriptions",
            post(subscriptions::create_subscription),
        )
        .route(
            "/api/ar/subscriptions/{id}",
            put(subscriptions::update_subscription),
        )
        .route(
            "/api/ar/subscriptions/{id}/cancel",
            post(subscriptions::cancel_subscription),
        )
        // Invoices — write
        .route("/api/ar/invoices", post(invoices::create_invoice))
        .route("/api/ar/invoices/{id}", put(invoices::update_invoice))
        .route(
            "/api/ar/invoices/{id}/finalize",
            post(invoices::finalize_invoice),
        )
        .route(
            "/api/ar/invoices/{id}/bill-usage",
            post(usage::bill_usage_route),
        )
        .route(
            "/api/ar/invoices/{id}/credit-notes",
            post(credit_notes::issue_credit_note_route),
        )
        .route(
            "/api/ar/credit-memos",
            post(credit_notes::create_credit_memo_handler),
        )
        .route(
            "/api/ar/credit-memos/{id}/approve",
            post(credit_notes::approve_credit_memo_handler),
        )
        .route(
            "/api/ar/credit-memos/{id}/issue",
            post(credit_notes::issue_credit_memo_handler),
        )
        .route(
            "/api/ar/invoices/{id}/write-off",
            post(write_offs::write_off_invoice_route),
        )
        // Charges — write
        .route("/api/ar/charges", post(charges::create_charge))
        .route(
            "/api/ar/charges/{id}/capture",
            post(charges::capture_charge),
        )
        // Refunds — write
        .route("/api/ar/refunds", post(refunds::create_refund))
        // Disputes — write
        .route(
            "/api/ar/disputes/{id}/evidence",
            post(disputes::submit_dispute_evidence),
        )
        // Payment methods — write
        .route(
            "/api/ar/payment-methods",
            post(payment_methods::add_payment_method),
        )
        .route(
            "/api/ar/payment-methods/{id}",
            put(payment_methods::update_payment_method)
                .delete(payment_methods::delete_payment_method),
        )
        .route(
            "/api/ar/payment-methods/{id}/set-default",
            post(payment_methods::set_default_payment_method),
        )
        // Webhook management — write
        .route(
            "/api/ar/webhooks/{id}/replay",
            post(webhooks::replay_webhook),
        )
        // Usage ingestion — write
        .route("/api/ar/usage", post(usage::capture_usage))
        // Aging refresh — write
        .route("/api/ar/aging/refresh", post(aging::refresh_aging_route))
        // Dunning — write
        .route(
            "/api/ar/dunning/poll",
            post(dunning_routes::dunning_poll_route),
        )
        // Reconciliation — write
        .route(
            "/api/ar/recon/run",
            post(reconciliation_routes::recon_run_route),
        )
        .route(
            "/api/ar/recon/schedule",
            post(reconciliation_routes::schedule_recon_route),
        )
        .route(
            "/api/ar/recon/poll",
            post(reconciliation_routes::recon_poll_route),
        )
        // Payment allocation — write
        .route(
            "/api/ar/payments/allocate",
            post(allocation::allocate_payment_route),
        )
        // Tax config — write
        .route(
            "/api/ar/tax/config/jurisdictions",
            post(tax_config::create_jurisdiction),
        )
        .route(
            "/api/ar/tax/config/jurisdictions/{id}",
            put(tax_config::update_jurisdiction),
        )
        .route(
            "/api/ar/tax/config/rules",
            post(tax_config_rules::create_rule),
        )
        .route(
            "/api/ar/tax/config/rules/{id}",
            put(tax_config_rules::update_rule),
        );

    let mutations = if enforce_permissions {
        mutations_core
            .route_layer(RequirePermissionsLayer::new(&[permissions::AR_MUTATE]))
            .with_state(db.clone())
    } else {
        mutations_core.with_state(db.clone())
    };

    // Read routes — in production require ar.read permission.
    let reads_core = Router::new()
        // Customers — read
        .route("/api/ar/customers", get(customers::list_customers))
        .route("/api/ar/customers/{id}", get(customers::get_customer))
        // Subscriptions — read
        .route(
            "/api/ar/subscriptions",
            get(subscriptions::list_subscriptions),
        )
        .route(
            "/api/ar/subscriptions/{id}",
            get(subscriptions::get_subscription),
        )
        // Invoices — read
        .route("/api/ar/invoices", get(invoices::list_invoices))
        .route("/api/ar/invoices/{id}", get(invoices::get_invoice))
        // Charges — read
        .route("/api/ar/charges", get(charges::list_charges))
        .route("/api/ar/charges/{id}", get(charges::get_charge))
        // Refunds — read
        .route("/api/ar/refunds", get(refunds::list_refunds))
        .route("/api/ar/refunds/{id}", get(refunds::get_refund))
        // Disputes — read
        .route("/api/ar/disputes", get(disputes::list_disputes))
        .route("/api/ar/disputes/{id}", get(disputes::get_dispute))
        // Payment methods — read
        .route(
            "/api/ar/payment-methods",
            get(payment_methods::list_payment_methods),
        )
        .route(
            "/api/ar/payment-methods/{id}",
            get(payment_methods::get_payment_method),
        )
        // Webhook management — read
        .route("/api/ar/webhooks", get(webhooks::list_webhooks))
        .route("/api/ar/webhooks/{id}", get(webhooks::get_webhook))
        // Event log — read
        .route("/api/ar/events", get(events::list_events))
        .route("/api/ar/events/{id}", get(events::get_event))
        // Aging report — read
        .route("/api/ar/aging", get(aging::get_aging))
        // Tax config — read
        .route(
            "/api/ar/tax/config/jurisdictions",
            get(tax_config::list_jurisdictions),
        )
        .route(
            "/api/ar/tax/config/jurisdictions/{id}",
            get(tax_config::get_jurisdiction),
        )
        .route(
            "/api/ar/tax/config/rules",
            get(tax_config_rules::list_rules),
        )
        .route(
            "/api/ar/tax/config/rules/{id}",
            get(tax_config_rules::get_rule),
        )
        // Tax reports — read
        .route(
            "/api/ar/tax/reports/summary",
            get(tax_reports::tax_report_summary),
        )
        .route(
            "/api/ar/tax/reports/export",
            get(tax_reports::tax_report_export),
        )
        ;

    let reads = if enforce_permissions {
        reads_core
            .route_layer(RequirePermissionsLayer::new(&[permissions::AR_READ]))
            .with_state(db.clone())
    } else {
        reads_core.with_state(db.clone())
    };

    Router::new()
        .merge(mutations)
        .merge(reads)
        .layer(middleware::from_fn_with_state(db, check_idempotency))
        .merge(webhook_inbound)
}
