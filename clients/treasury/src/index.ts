// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import type { paths, components } from "./treasury.d.ts";

export type { paths, components } from "./treasury.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AccountPosition = components["schemas"]["AccountPosition"];
export type AccountStatus = components["schemas"]["AccountStatus"];
export type AccountType = components["schemas"]["AccountType"];
export type ApiError = components["schemas"]["ApiError"];
export type AutoMatchRequest = components["schemas"]["AutoMatchRequest"];
export type AutoMatchResult = components["schemas"]["AutoMatchResult"];
export type CashPositionResponse = components["schemas"]["CashPositionResponse"];
export type CashPositionSummary = components["schemas"]["CashPositionSummary"];
export type CreateBankAccountRequest = components["schemas"]["CreateBankAccountRequest"];
export type CreateCreditCardAccountRequest = components["schemas"]["CreateCreditCardAccountRequest"];
export type CurrencyForecast = components["schemas"]["CurrencyForecast"];
export type FieldError = components["schemas"]["FieldError"];
export type ForecastAssumptions = components["schemas"]["ForecastAssumptions"];
export type ForecastBuckets = components["schemas"]["ForecastBuckets"];
export type ForecastResponse = components["schemas"]["ForecastResponse"];
export type GlLinkResponse = components["schemas"]["GlLinkResponse"];
export type ImportResult = components["schemas"]["ImportResult"];
export type LineError = components["schemas"]["LineError"];
export type LinkToGlRequest = components["schemas"]["LinkToGlRequest"];
export type ManualMatchRequest = components["schemas"]["ManualMatchRequest"];
export type PaginatedResponse_TreasuryAccount = components["schemas"]["PaginatedResponse_TreasuryAccount"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type ReconMatch = components["schemas"]["ReconMatch"];
export type ReconMatchStatus = components["schemas"]["ReconMatchStatus"];
export type ReconMatchType = components["schemas"]["ReconMatchType"];
export type TreasuryAccount = components["schemas"]["TreasuryAccount"];
export type UnmatchedBankTxnGl = components["schemas"]["UnmatchedBankTxnGl"];
export type UnmatchedBankTxnsResponse = components["schemas"]["UnmatchedBankTxnsResponse"];
export type UnmatchedGlRequest = components["schemas"]["UnmatchedGlRequest"];
export type UnmatchedGlResult = components["schemas"]["UnmatchedGlResult"];
export type UpdateAccountRequest = components["schemas"]["UpdateAccountRequest"];

export interface TreasuryClientOptions {
  baseUrl: string;
  token: string;
}

export function createTreasuryClient(opts: TreasuryClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
