// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./fixed-assets.d.ts";

export type { paths, components } from "./fixed-assets.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type ApiError = components["schemas"]["ApiError"];
export type Asset = components["schemas"]["Asset"];
export type AssetStatus = components["schemas"]["AssetStatus"];
export type Category = components["schemas"]["Category"];
export type CreateAssetRequest = components["schemas"]["CreateAssetRequest"];
export type CreateCategoryRequest = components["schemas"]["CreateCategoryRequest"];
export type CreateRunRequest = components["schemas"]["CreateRunRequest"];
export type DepreciationMethod = components["schemas"]["DepreciationMethod"];
export type DepreciationRun = components["schemas"]["DepreciationRun"];
export type DepreciationSchedule = components["schemas"]["DepreciationSchedule"];
export type Disposal = components["schemas"]["Disposal"];
export type DisposalType = components["schemas"]["DisposalType"];
export type DisposeAssetRequest = components["schemas"]["DisposeAssetRequest"];
export type FieldError = components["schemas"]["FieldError"];
export type GenerateScheduleRequest = components["schemas"]["GenerateScheduleRequest"];
export type PaginatedResponse_Asset = components["schemas"]["PaginatedResponse_Asset"];
export type PaginatedResponse_Category = components["schemas"]["PaginatedResponse_Category"];
export type PaginatedResponse_DepreciationRun = components["schemas"]["PaginatedResponse_DepreciationRun"];
export type PaginatedResponse_Disposal = components["schemas"]["PaginatedResponse_Disposal"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type UpdateAssetRequest = components["schemas"]["UpdateAssetRequest"];
export type UpdateCategoryRequest = components["schemas"]["UpdateCategoryRequest"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type FixedAssetsClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createFixedAssetsClient(opts: FixedAssetsClientOptions) {
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
