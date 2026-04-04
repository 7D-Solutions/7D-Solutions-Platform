pub mod bill_run;
pub mod health;

use axum::{routing::post, Router};
use sqlx::PgPool;
use utoipa::OpenApi;

pub use bill_run::execute_bill_run;
pub use health::{health, ready, version};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Subscriptions Service",
        version = "5.0.0",
        description = "Subscription lifecycle management, billing cycles, and usage gating.",
    ),
    paths(
        bill_run::execute_bill_run,
        health::health,
        health::ready,
        health::version,
        crate::admin::projection_status,
        crate::admin::consistency_check,
        crate::admin::list_projections,
    ),
    components(schemas(
        crate::models::SubscriptionPlan,
        crate::models::CreateSubscriptionPlanRequest,
        crate::models::Subscription,
        crate::models::CreateSubscriptionRequest,
        crate::models::PauseSubscriptionRequest,
        crate::models::CancelSubscriptionRequest,
        crate::models::BillRun,
        crate::models::ExecuteBillRunRequest,
        crate::models::BillRunResult,
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

pub fn subscriptions_router(db: PgPool) -> Router {
    Router::new()
        .route("/api/bill-runs/execute", post(execute_bill_run))
        .with_state(db)
}
