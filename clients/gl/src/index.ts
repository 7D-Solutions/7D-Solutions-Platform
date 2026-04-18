// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./gl.d.ts";

export type { paths, components } from "./gl.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AccountActivityLine = components["schemas"]["AccountActivityLine"];
export type AccountActivityResponse = components["schemas"]["AccountActivityResponse"];
export type AccountResponse = components["schemas"]["AccountResponse"];
export type AccountType = components["schemas"]["AccountType"];
export type AccrualResult = components["schemas"]["AccrualResult"];
export type AllocationChange = components["schemas"]["AllocationChange"];
export type AmendContractResponse = components["schemas"]["AmendContractResponse"];
export type ApiError = components["schemas"]["ApiError"];
export type ApprovalResponse = components["schemas"]["ApprovalResponse"];
export type BalanceSheetResponse = components["schemas"]["BalanceSheetResponse"];
export type BalanceSheetRow = components["schemas"]["BalanceSheetRow"];
export type BalanceSheetTotals = components["schemas"]["BalanceSheetTotals"];
export type CashFlowCategoryTotal = components["schemas"]["CashFlowCategoryTotal"];
export type CashFlowResponse = components["schemas"]["CashFlowResponse"];
export type CashFlowRow = components["schemas"]["CashFlowRow"];
export type ChecklistItemResponse = components["schemas"]["ChecklistItemResponse"];
export type ClosePeriodRequest = components["schemas"]["ClosePeriodRequest"];
export type ClosePeriodResponse = components["schemas"]["ClosePeriodResponse"];
export type CloseStatus = components["schemas"]["CloseStatus"];
export type CloseStatusResponse = components["schemas"]["CloseStatusResponse"];
export type CompleteChecklistItemRequest = components["schemas"]["CompleteChecklistItemRequest"];
export type ContractModifiedPayload = components["schemas"]["ContractModifiedPayload"];
export type CreateAccountRequest = components["schemas"]["CreateAccountRequest"];
export type CreateAccrualRequest = components["schemas"]["CreateAccrualRequest"];
export type CreateApprovalRequest = components["schemas"]["CreateApprovalRequest"];
export type CreateChecklistItemRequest = components["schemas"]["CreateChecklistItemRequest"];
export type CreateContractRequest = components["schemas"]["CreateContractRequest"];
export type CreateContractResponse = components["schemas"]["CreateContractResponse"];
export type CreateExportRequest = components["schemas"]["CreateExportRequest"];
export type CreateFxRateRequest = components["schemas"]["CreateFxRateRequest"];
export type CreateFxRateResponse = components["schemas"]["CreateFxRateResponse"];
export type CreateTemplateRequest = components["schemas"]["CreateTemplateRequest"];
export type ExecuteReversalsRequest = components["schemas"]["ExecuteReversalsRequest"];
export type ExecuteReversalsResult = components["schemas"]["ExecuteReversalsResult"];
export type ExportResponse = components["schemas"]["ExportResponse"];
export type FieldError = components["schemas"]["FieldError"];
export type FxRateResponse = components["schemas"]["FxRateResponse"];
export type GLDetailEntry = components["schemas"]["GLDetailEntry"];
export type GLDetailEntryLine = components["schemas"]["GLDetailEntryLine"];
export type GLDetailResponse = components["schemas"]["GLDetailResponse"];
export type GenerateScheduleRequest = components["schemas"]["GenerateScheduleRequest"];
export type GenerateScheduleResponse = components["schemas"]["GenerateScheduleResponse"];
export type IncomeStatementResponse = components["schemas"]["IncomeStatementResponse"];
export type IncomeStatementRow = components["schemas"]["IncomeStatementRow"];
export type IncomeStatementTotals = components["schemas"]["IncomeStatementTotals"];
export type ModificationType = components["schemas"]["ModificationType"];
export type NormalBalance = components["schemas"]["NormalBalance"];
export type PaginationMetadata = components["schemas"]["PaginationMetadata"];
export type PerformanceObligation = components["schemas"]["PerformanceObligation"];
export type PeriodSummaryResponse = components["schemas"]["PeriodSummaryResponse"];
export type RecognitionPattern = components["schemas"]["RecognitionPattern"];
export type RecognitionPostingResponse = components["schemas"]["RecognitionPostingResponse"];
export type RecognitionRunRequest = components["schemas"]["RecognitionRunRequest"];
export type RecognitionRunResponse = components["schemas"]["RecognitionRunResponse"];
export type ReopenApprovePayload = components["schemas"]["ReopenApprovePayload"];
export type ReopenRejectPayload = components["schemas"]["ReopenRejectPayload"];
export type ReopenRequestPayload = components["schemas"]["ReopenRequestPayload"];
export type ReportingBalanceSheetResponse = components["schemas"]["ReportingBalanceSheetResponse"];
export type ReportingIncomeStatementResponse = components["schemas"]["ReportingIncomeStatementResponse"];
export type ReportingTrialBalanceResponse = components["schemas"]["ReportingTrialBalanceResponse"];
export type ReversalPolicy = components["schemas"]["ReversalPolicy"];
export type ReversalResult = components["schemas"]["ReversalResult"];
export type StatementTotals = components["schemas"]["StatementTotals"];
export type TemplateResult = components["schemas"]["TemplateResult"];
export type TrialBalanceResponse = components["schemas"]["TrialBalanceResponse"];
export type TrialBalanceRow = components["schemas"]["TrialBalanceRow"];
export type ValidateCloseRequest = components["schemas"]["ValidateCloseRequest"];
export type ValidateCloseResponse = components["schemas"]["ValidateCloseResponse"];
export type ValidationIssue = components["schemas"]["ValidationIssue"];
export type ValidationReport = components["schemas"]["ValidationReport"];
export type ValidationSeverity = components["schemas"]["ValidationSeverity"];
export type WaiveChecklistItemRequest = components["schemas"]["WaiveChecklistItemRequest"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type GlClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createGlClient(opts: GlClientOptions) {
  if ("authClient" in opts) {
    const client = createClient<paths>({ baseUrl: opts.baseUrl });
    client.use(createAuthMiddleware(opts.authClient));
    return client;
  }
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
