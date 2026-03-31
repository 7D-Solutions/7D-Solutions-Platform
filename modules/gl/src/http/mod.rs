pub mod account_activity;
pub mod accounts;
pub mod accruals;
pub mod admin;
pub mod auth;
pub mod balance_sheet;
pub mod cashflow;
pub mod close_checklist;
pub mod exports;
pub mod fx_rates;
pub mod gl_detail;
pub mod health;
pub mod income_statement;
pub mod period_close;
pub mod period_summary;
pub mod reporting_currency;
pub mod revrec;
pub mod trial_balance;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "GL Service",
        version = "2.0.2",
        description = "Double-entry general ledger with journal engine, accruals, and revenue recognition.",
    ),
    paths(
        accounts::create_account,
        accruals::create_template_handler,
        accruals::create_accrual_handler,
        accruals::execute_reversals_handler,
        trial_balance::get_trial_balance,
        balance_sheet::get_balance_sheet,
        income_statement::get_income_statement,
        cashflow::get_cash_flow,
        period_summary::get_period_summary,
        account_activity::get_account_activity,
        gl_detail::get_gl_detail,
        exports::create_export,
        fx_rates::create_fx_rate,
        fx_rates::get_latest_rate,
        reporting_currency::get_reporting_trial_balance,
        reporting_currency::get_reporting_income_statement,
        reporting_currency::get_reporting_balance_sheet,
        period_close::validate_close,
        period_close::close_period_handler,
        period_close::get_close_status,
        period_close::request_reopen,
        period_close::approve_reopen,
        period_close::reject_reopen,
        period_close::list_reopen_requests,
        close_checklist::create_checklist_item,
        close_checklist::complete_checklist_item,
        close_checklist::waive_checklist_item,
        close_checklist::get_checklist_status,
        close_checklist::create_approval,
        close_checklist::get_approvals,
        revrec::create_contract,
        revrec::generate_schedule_handler,
        revrec::run_recognition_handler,
        revrec::amend_contract,
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
