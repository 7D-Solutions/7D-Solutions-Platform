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
pub mod journal_entries;
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
        version = "3.1.0",
        description = "Double-entry general ledger with journal engine, accruals, and revenue recognition.",
    ),
    paths(
        // Health
        health::health,
        health::ready,
        health::version,
        // Admin
        admin::projection_status,
        admin::consistency_check,
        admin::list_projections,
        // Accounts
        accounts::create_account,
        // Accruals
        accruals::create_template_handler,
        accruals::create_accrual_handler,
        accruals::execute_reversals_handler,
        // Financial statements
        trial_balance::get_trial_balance,
        balance_sheet::get_balance_sheet,
        income_statement::get_income_statement,
        cashflow::get_cash_flow,
        period_summary::get_period_summary,
        account_activity::get_account_activity,
        gl_detail::get_gl_detail,
        // Exports & FX
        exports::create_export,
        fx_rates::create_fx_rate,
        fx_rates::get_latest_rate,
        // Reporting currency
        reporting_currency::get_reporting_trial_balance,
        reporting_currency::get_reporting_income_statement,
        reporting_currency::get_reporting_balance_sheet,
        // Period close
        period_close::validate_close,
        period_close::close_period_handler,
        period_close::get_close_status,
        period_close::request_reopen,
        period_close::approve_reopen,
        period_close::reject_reopen,
        period_close::list_reopen_requests,
        // Close checklist
        close_checklist::create_checklist_item,
        close_checklist::complete_checklist_item,
        close_checklist::waive_checklist_item,
        close_checklist::get_checklist_status,
        close_checklist::create_approval,
        close_checklist::get_approvals,
        // Journal Entries
        journal_entries::create_journal_entry,
        // Revenue recognition
        revrec::create_contract,
        revrec::generate_schedule_handler,
        revrec::run_recognition_handler,
        revrec::amend_contract,
    ),
    components(schemas(
        platform_http_contracts::ApiError,
        platform_http_contracts::PaginatedResponse<close_checklist::ChecklistItemResponse>,
        platform_http_contracts::PaginatedResponse<close_checklist::ApprovalResponse>,
        platform_http_contracts::PaginatedResponse<period_close::ReopenRequestResponse>,
        platform_http_contracts::PaginationMeta,
        // Accounts
        accounts::CreateAccountRequest,
        accounts::AccountResponse,
        crate::repos::account_repo::AccountType,
        crate::repos::account_repo::NormalBalance,
        // Financial Statements
        crate::services::trial_balance_service::TrialBalanceResponse,
        crate::services::balance_sheet_service::BalanceSheetResponse,
        crate::services::balance_sheet_service::BalanceSheetTotals,
        crate::services::income_statement_service::IncomeStatementResponse,
        crate::services::income_statement_service::IncomeStatementTotals,
        crate::services::cashflow_service::CashFlowResponse,
        crate::services::period_summary_service::PeriodSummaryResponse,
        crate::services::account_activity_service::AccountActivityResponse,
        crate::services::account_activity_service::AccountActivityLine,
        crate::services::account_activity_service::PaginationMetadata,
        crate::services::gl_detail_service::GLDetailResponse,
        crate::services::gl_detail_service::GLDetailEntry,
        crate::services::gl_detail_service::GLDetailEntryLine,
        // Journal Entries
        journal_entries::PostJournalEntryRequest,
        journal_entries::PostJournalEntryResponse,
        crate::contracts::gl_posting_request_v1::SourceDocType,
        crate::contracts::gl_posting_request_v1::JournalLine,
        // Domain types
        crate::domain::statements::TrialBalanceRow,
        crate::domain::statements::BalanceSheetRow,
        crate::domain::statements::IncomeStatementRow,
        crate::domain::statements::StatementTotals,
        crate::domain::statements::CashFlowRow,
        crate::domain::statements::CashFlowCategoryTotal,
        // Reporting Currency
        reporting_currency::ReportingTrialBalanceResponse,
        reporting_currency::ReportingIncomeStatementResponse,
        reporting_currency::ReportingBalanceSheetResponse,
        // Exports
        exports::CreateExportRequest,
        exports::ExportResponse,
        // FX Rates
        fx_rates::CreateFxRateRequest,
        fx_rates::CreateFxRateResponse,
        fx_rates::FxRateResponse,
        // Period Close
        crate::contracts::period_close_v1::ValidateCloseRequest,
        crate::contracts::period_close_v1::ValidateCloseResponse,
        crate::contracts::period_close_v1::ClosePeriodRequest,
        crate::contracts::period_close_v1::ClosePeriodResponse,
        crate::contracts::period_close_v1::CloseStatusResponse,
        crate::contracts::period_close_v1::CloseStatus,
        crate::contracts::period_close_v1::ValidationReport,
        crate::contracts::period_close_v1::ValidationIssue,
        crate::contracts::period_close_v1::ValidationSeverity,
        period_close::ReopenRequestPayload,
        period_close::ReopenApprovePayload,
        period_close::ReopenRejectPayload,
        period_close::ReopenRequestResponse,
        // Close Checklist
        close_checklist::CreateChecklistItemRequest,
        close_checklist::ChecklistItemResponse,
        close_checklist::CompleteChecklistItemRequest,
        close_checklist::WaiveChecklistItemRequest,
        close_checklist::CreateApprovalRequest,
        close_checklist::ApprovalResponse,
        // Accruals
        crate::accruals::CreateTemplateRequest,
        crate::accruals::TemplateResult,
        crate::accruals::CreateAccrualRequest,
        crate::accruals::AccrualResult,
        crate::accruals::ExecuteReversalsRequest,
        crate::accruals::ExecuteReversalsResult,
        crate::accruals::ReversalResult,
        crate::events::contracts::accruals::ReversalPolicy,
        // Revenue Recognition
        revrec::CreateContractRequest,
        revrec::CreateContractResponse,
        revrec::GenerateScheduleRequest,
        revrec::GenerateScheduleResponse,
        revrec::RecognitionRunRequest,
        revrec::RecognitionRunResponse,
        revrec::RecognitionPostingResponse,
        revrec::AmendContractResponse,
        crate::revrec::PerformanceObligation,
        crate::revrec::RecognitionPattern,
        crate::revrec::ContractModifiedPayload,
        crate::revrec::AllocationChange,
        crate::revrec::ModificationType,
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
