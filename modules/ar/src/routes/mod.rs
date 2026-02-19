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
pub mod usage;
pub mod webhooks;
pub mod write_offs;

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use sqlx::PgPool;

use crate::idempotency::check_idempotency;

pub fn ar_router(db: PgPool) -> Router {
    Router::new()
        // Customer endpoints
        .route("/api/ar/customers", post(customers::create_customer).get(customers::list_customers))
        .route(
            "/api/ar/customers/{id}",
            get(customers::get_customer).put(customers::update_customer),
        )
        // Subscription endpoints
        .route(
            "/api/ar/subscriptions",
            post(subscriptions::create_subscription).get(subscriptions::list_subscriptions),
        )
        .route(
            "/api/ar/subscriptions/{id}",
            get(subscriptions::get_subscription).put(subscriptions::update_subscription),
        )
        .route(
            "/api/ar/subscriptions/{id}/cancel",
            post(subscriptions::cancel_subscription),
        )
        // Invoice endpoints
        .route("/api/ar/invoices", post(invoices::create_invoice).get(invoices::list_invoices))
        .route(
            "/api/ar/invoices/{id}",
            get(invoices::get_invoice).put(invoices::update_invoice),
        )
        .route("/api/ar/invoices/{id}/finalize", post(invoices::finalize_invoice))
        .route("/api/ar/invoices/{id}/bill-usage", post(usage::bill_usage_route))
        .route("/api/ar/invoices/{id}/credit-notes", post(credit_notes::issue_credit_note_route))
        .route("/api/ar/invoices/{id}/write-off", post(write_offs::write_off_invoice_route))
        // Charge endpoints
        .route("/api/ar/charges", post(charges::create_charge).get(charges::list_charges))
        .route("/api/ar/charges/{id}", get(charges::get_charge))
        .route("/api/ar/charges/{id}/capture", post(charges::capture_charge))
        // Refund endpoints
        .route("/api/ar/refunds", post(refunds::create_refund).get(refunds::list_refunds))
        .route("/api/ar/refunds/{id}", get(refunds::get_refund))
        // Dispute endpoints
        .route("/api/ar/disputes", get(disputes::list_disputes))
        .route("/api/ar/disputes/{id}", get(disputes::get_dispute))
        .route("/api/ar/disputes/{id}/evidence", post(disputes::submit_dispute_evidence))
        // Payment method endpoints
        .route(
            "/api/ar/payment-methods",
            post(payment_methods::add_payment_method).get(payment_methods::list_payment_methods),
        )
        .route(
            "/api/ar/payment-methods/{id}",
            get(payment_methods::get_payment_method)
                .put(payment_methods::update_payment_method)
                .delete(payment_methods::delete_payment_method),
        )
        .route(
            "/api/ar/payment-methods/{id}/set-default",
            post(payment_methods::set_default_payment_method),
        )
        // Webhook endpoints
        .route("/api/ar/webhooks/tilled", post(webhooks::receive_tilled_webhook))
        .route("/api/ar/webhooks", get(webhooks::list_webhooks))
        .route("/api/ar/webhooks/{id}", get(webhooks::get_webhook))
        .route("/api/ar/webhooks/{id}/replay", post(webhooks::replay_webhook))
        // Event log endpoints
        .route("/api/ar/events", get(events::list_events))
        .route("/api/ar/events/{id}", get(events::get_event))
        // Usage ingestion (bd-23z)
        .route("/api/ar/usage", post(usage::capture_usage))
        // AR aging report (bd-3cb)
        .route("/api/ar/aging", get(aging::get_aging))
        .route("/api/ar/aging/refresh", post(aging::refresh_aging_route))
        // Dunning scheduler (bd-2bj)
        .route("/api/ar/dunning/poll", post(dunning_routes::dunning_poll_route))
        // Reconciliation matching (bd-2cn)
        .route("/api/ar/recon/run", post(reconciliation_routes::recon_run_route))
        // Scheduled reconciliation runs (bd-1kl)
        .route("/api/ar/recon/schedule", post(reconciliation_routes::schedule_recon_route))
        .route("/api/ar/recon/poll", post(reconciliation_routes::recon_poll_route))
        // Payment allocation (bd-14f)
        .route("/api/ar/payments/allocate", post(allocation::allocate_payment_route))
        // Tax config CRUD (bd-1m3c)
        .route(
            "/api/ar/tax/config/jurisdictions",
            post(tax_config::create_jurisdiction).get(tax_config::list_jurisdictions),
        )
        .route(
            "/api/ar/tax/config/jurisdictions/{id}",
            get(tax_config::get_jurisdiction).put(tax_config::update_jurisdiction),
        )
        .route(
            "/api/ar/tax/config/rules",
            post(tax_config::create_rule).get(tax_config::list_rules),
        )
        .route(
            "/api/ar/tax/config/rules/{id}",
            get(tax_config::get_rule).put(tax_config::update_rule),
        )
        .with_state(db.clone())
        .layer(middleware::from_fn_with_state(db, check_idempotency))
}
