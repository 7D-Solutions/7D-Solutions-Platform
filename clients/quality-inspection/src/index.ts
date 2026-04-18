// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./quality-inspection.d.ts";

export type { paths, components } from "./quality-inspection.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type ApiError = components["schemas"]["ApiError"];
export type Characteristic = components["schemas"]["Characteristic"];
export type CreateFinalInspectionRequest = components["schemas"]["CreateFinalInspectionRequest"];
export type CreateInProcessInspectionRequest = components["schemas"]["CreateInProcessInspectionRequest"];
export type CreateInspectionPlanRequest = components["schemas"]["CreateInspectionPlanRequest"];
export type CreateReceivingInspectionRequest = components["schemas"]["CreateReceivingInspectionRequest"];
export type DispositionTransitionRequest = components["schemas"]["DispositionTransitionRequest"];
export type FieldError = components["schemas"]["FieldError"];
export type Inspection = components["schemas"]["Inspection"];
export type InspectionPlan = components["schemas"]["InspectionPlan"];
export type PaginatedResponse_Inspection = components["schemas"]["PaginatedResponse_Inspection"];
export type PaginatedResponse_InspectionPlan = components["schemas"]["PaginatedResponse_InspectionPlan"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type QualityInspectionClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createQualityInspectionClient(opts: QualityInspectionClientOptions) {
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
