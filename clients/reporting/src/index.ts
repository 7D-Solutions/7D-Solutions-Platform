// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import type { paths, components } from "./reporting.d.ts";

export type { paths, components } from "./reporting.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type ApAgingReport = components["schemas"]["ApAgingReport"];
export type ApiError = components["schemas"]["ApiError"];
export type ArAgingResponse = components["schemas"]["ArAgingResponse"];
export type ArAgingRow = components["schemas"]["ArAgingRow"];
export type ArAgingSummary = components["schemas"]["ArAgingSummary"];
export type AtRiskItem = components["schemas"]["AtRiskItem"];
export type BTreeMap = components["schemas"]["BTreeMap"];
export type BalanceSheet = components["schemas"]["BalanceSheet"];
export type BsAccountLine = components["schemas"]["BsAccountLine"];
export type BsSection = components["schemas"]["BsSection"];
export type CashForecastResponse = components["schemas"]["CashForecastResponse"];
export type CashflowLine = components["schemas"]["CashflowLine"];
export type CashflowSection = components["schemas"]["CashflowSection"];
export type CashflowStatement = components["schemas"]["CashflowStatement"];
export type CurrencyForecast = components["schemas"]["CurrencyForecast"];
export type CurrencySummary = components["schemas"]["CurrencySummary"];
export type FieldError = components["schemas"]["FieldError"];
export type ForecastHorizon = components["schemas"]["ForecastHorizon"];
export type KpiSnapshot = components["schemas"]["KpiSnapshot"];
export type PlAccountLine = components["schemas"]["PlAccountLine"];
export type PlSection = components["schemas"]["PlSection"];
export type PlStatement = components["schemas"]["PlStatement"];
export type RebuildRequest = components["schemas"]["RebuildRequest"];
export type SnapshotRunResult = components["schemas"]["SnapshotRunResult"];
export type VendorAgingRow = components["schemas"]["VendorAgingRow"];

export interface ReportingClientOptions {
  baseUrl: string;
  token: string;
}

export function createReportingClient(opts: ReportingClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
