// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./platform-client-tenant-registry.d.ts";

export type { paths, components } from "./platform-client-tenant-registry.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type TenantSummaryDto = components["schemas"]["TenantSummaryDto"];
export type TenantListResponse = components["schemas"]["TenantListResponse"];
export type TenantDetailDto = components["schemas"]["TenantDetailDto"];
export type EntitlementRow = components["schemas"]["EntitlementRow"];
export type TenantAppIdRow = components["schemas"]["TenantAppIdRow"];
export type TenantStatusRow = components["schemas"]["TenantStatusRow"];
export type TenantSummary = components["schemas"]["TenantSummary"];
export type ModuleReadiness = components["schemas"]["ModuleReadiness"];
export type PlanSummary = components["schemas"]["PlanSummary"];
export type PlanListResponse = components["schemas"]["PlanListResponse"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type PlatformClientTenantRegistryClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createPlatformClientTenantRegistryClient(opts: PlatformClientTenantRegistryClientOptions) {
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
