use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Payments Service",
        version = "3.0.0",
        description = "Payment processing: checkout sessions, payment retrieval, and Tilled webhooks.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims.",
    ),
    paths(
        payments_rs::http::checkout_sessions::create_checkout_session,
        payments_rs::http::checkout_sessions::get_checkout_session,
        payments_rs::http::checkout_sessions::present_checkout_session,
        payments_rs::http::checkout_sessions::poll_checkout_session_status,
        payments_rs::http::checkout_sessions::tilled_webhook,
        payments_rs::http::payments::get_payment,
        payments_rs::http::health::health,
        payments_rs::http::health::ready,
        payments_rs::http::health::version,
        payments_rs::http::admin::projection_status,
        payments_rs::http::admin::consistency_check,
        payments_rs::http::admin::list_projections,
    ),
    components(schemas(
        payments_rs::http::checkout_sessions::CreateCheckoutSessionRequest,
        payments_rs::http::checkout_sessions::CreateCheckoutSessionResponse,
        payments_rs::http::checkout_sessions::CheckoutSessionStatusResponse,
        payments_rs::http::checkout_sessions::SessionStatusPollResponse,
        payments_rs::http::payments::PaymentResponse,
        payments_rs::http::payments::DataSource,
        platform_http_contracts::ApiError,
        platform_http_contracts::FieldError,
        platform_http_contracts::PaginationMeta,
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
                utoipa::openapi::security::Http::new(
                    utoipa::openapi::security::HttpAuthScheme::Bearer,
                ),
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
