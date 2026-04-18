// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./consolidation.d.ts";

export type { paths, components } from "./consolidation.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type ApiError = components["schemas"]["ApiError"];
export type BsAccountLine = components["schemas"]["BsAccountLine"];
export type BsSection = components["schemas"]["BsSection"];
export type CachedTbResponse = components["schemas"]["CachedTbResponse"];
export type CachedTbRow = components["schemas"]["CachedTbRow"];
export type CoaMapping = components["schemas"]["CoaMapping"];
export type ConsolidateQuery = components["schemas"]["ConsolidateQuery"];
export type ConsolidateResponse = components["schemas"]["ConsolidateResponse"];
export type ConsolidatedBalanceSheet = components["schemas"]["ConsolidatedBalanceSheet"];
export type ConsolidatedPl = components["schemas"]["ConsolidatedPl"];
export type ConsolidatedTbRow = components["schemas"]["ConsolidatedTbRow"];
export type CreateCoaMappingRequest = components["schemas"]["CreateCoaMappingRequest"];
export type CreateEliminationRuleRequest = components["schemas"]["CreateEliminationRuleRequest"];
export type CreateEntityRequest = components["schemas"]["CreateEntityRequest"];
export type CreateGroupRequest = components["schemas"]["CreateGroupRequest"];
export type EliminationRule = components["schemas"]["EliminationRule"];
export type EliminationSuggestion = components["schemas"]["EliminationSuggestion"];
export type EntityHashEntry = components["schemas"]["EntityHashEntry"];
export type FieldError = components["schemas"]["FieldError"];
export type FxPolicy = components["schemas"]["FxPolicy"];
export type Group = components["schemas"]["Group"];
export type GroupEntity = components["schemas"]["GroupEntity"];
export type IntercompanyMatch = components["schemas"]["IntercompanyMatch"];
export type IntercompanyMatchRequest = components["schemas"]["IntercompanyMatchRequest"];
export type IntercompanyMatchResponse = components["schemas"]["IntercompanyMatchResponse"];
export type PaginatedResponse_CoaMapping = components["schemas"]["PaginatedResponse_CoaMapping"];
export type PaginatedResponse_EliminationRule = components["schemas"]["PaginatedResponse_EliminationRule"];
export type PaginatedResponse_FxPolicy = components["schemas"]["PaginatedResponse_FxPolicy"];
export type PaginatedResponse_Group = components["schemas"]["PaginatedResponse_Group"];
export type PaginatedResponse_GroupEntity = components["schemas"]["PaginatedResponse_GroupEntity"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type PlAccountLine = components["schemas"]["PlAccountLine"];
export type PlSection = components["schemas"]["PlSection"];
export type PostEliminationsRequest = components["schemas"]["PostEliminationsRequest"];
export type PostEliminationsResponse = components["schemas"]["PostEliminationsResponse"];
export type UpdateEliminationRuleRequest = components["schemas"]["UpdateEliminationRuleRequest"];
export type UpdateEntityRequest = components["schemas"]["UpdateEntityRequest"];
export type UpdateGroupRequest = components["schemas"]["UpdateGroupRequest"];
export type UpsertFxPolicyRequest = components["schemas"]["UpsertFxPolicyRequest"];
export type ValidationResult = components["schemas"]["ValidationResult"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type ConsolidationClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createConsolidationClient(opts: ConsolidationClientOptions) {
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
