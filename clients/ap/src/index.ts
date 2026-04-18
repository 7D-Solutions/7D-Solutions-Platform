// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./ap.d.ts";

export type { paths, components } from "./ap.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AgingReport = components["schemas"]["AgingReport"];
export type AllocationRecord = components["schemas"]["AllocationRecord"];
export type ApTaxReportResponse = components["schemas"]["ApTaxReportResponse"];
export type ApTaxSnapshot = components["schemas"]["ApTaxSnapshot"];
export type ApTaxSummaryRow = components["schemas"]["ApTaxSummaryRow"];
export type ApiError = components["schemas"]["ApiError"];
export type ApproveBillRequest = components["schemas"]["ApproveBillRequest"];
export type ApprovePoRequest = components["schemas"]["ApprovePoRequest"];
export type AssignTermsRequest = components["schemas"]["AssignTermsRequest"];
export type BillBalanceSummary = components["schemas"]["BillBalanceSummary"];
export type BillLineRecord = components["schemas"]["BillLineRecord"];
export type CreateAllocationRequest = components["schemas"]["CreateAllocationRequest"];
export type CreateBillLineRequest = components["schemas"]["CreateBillLineRequest"];
export type CreateBillRequest = components["schemas"]["CreateBillRequest"];
export type CreatePaymentRunBody = components["schemas"]["CreatePaymentRunBody"];
export type CreatePaymentTermsRequest = components["schemas"]["CreatePaymentTermsRequest"];
export type CreatePoLineRequest = components["schemas"]["CreatePoLineRequest"];
export type CreatePoRequest = components["schemas"]["CreatePoRequest"];
export type CreateVendorRequest = components["schemas"]["CreateVendorRequest"];
export type CurrencyBucket = components["schemas"]["CurrencyBucket"];
export type FieldError = components["schemas"]["FieldError"];
export type MatchLineResult = components["schemas"]["MatchLineResult"];
export type MatchOutcome = components["schemas"]["MatchOutcome"];
export type PaginatedResponse_AllocationRecord = components["schemas"]["PaginatedResponse_AllocationRecord"];
export type PaginatedResponse_PaymentTerms = components["schemas"]["PaginatedResponse_PaymentTerms"];
export type PaginatedResponse_PurchaseOrder = components["schemas"]["PaginatedResponse_PurchaseOrder"];
export type PaginatedResponse_Vendor = components["schemas"]["PaginatedResponse_Vendor"];
export type PaginatedResponse_VendorBill = components["schemas"]["PaginatedResponse_VendorBill"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type PaymentRun = components["schemas"]["PaymentRun"];
export type PaymentTerms = components["schemas"]["PaymentTerms"];
export type PoLineRecord = components["schemas"]["PoLineRecord"];
export type PurchaseOrder = components["schemas"]["PurchaseOrder"];
export type PurchaseOrderWithLines = components["schemas"]["PurchaseOrderWithLines"];
export type RunMatchRequest = components["schemas"]["RunMatchRequest"];
export type UpdatePaymentTermsRequest = components["schemas"]["UpdatePaymentTermsRequest"];
export type UpdatePoLinesRequest = components["schemas"]["UpdatePoLinesRequest"];
export type UpdateVendorRequest = components["schemas"]["UpdateVendorRequest"];
export type Vendor = components["schemas"]["Vendor"];
export type VendorBill = components["schemas"]["VendorBill"];
export type VendorBillWithLines = components["schemas"]["VendorBillWithLines"];
export type VendorBucket = components["schemas"]["VendorBucket"];
export type VoidBillRequest = components["schemas"]["VoidBillRequest"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type ApClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createApClient(opts: ApClientOptions) {
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
