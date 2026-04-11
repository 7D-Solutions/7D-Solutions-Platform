// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import type { paths, components } from "./bom.d.ts";

export type { paths, components } from "./bom.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AddLineRequest = components["schemas"]["AddLineRequest"];
export type ApiError = components["schemas"]["ApiError"];
export type ApplyEcoRequest = components["schemas"]["ApplyEcoRequest"];
export type BomHeader = components["schemas"]["BomHeader"];
export type BomLine = components["schemas"]["BomLine"];
export type BomRevision = components["schemas"]["BomRevision"];
export type CreateBomRequest = components["schemas"]["CreateBomRequest"];
export type CreateEcoRequest = components["schemas"]["CreateEcoRequest"];
export type CreateRevisionRequest = components["schemas"]["CreateRevisionRequest"];
export type Eco = components["schemas"]["Eco"];
export type EcoActionRequest = components["schemas"]["EcoActionRequest"];
export type EcoAuditEntry = components["schemas"]["EcoAuditEntry"];
export type EcoBomRevision = components["schemas"]["EcoBomRevision"];
export type EcoDocRevision = components["schemas"]["EcoDocRevision"];
export type ExplosionRow = components["schemas"]["ExplosionRow"];
export type FieldError = components["schemas"]["FieldError"];
export type LinkBomRevisionRequest = components["schemas"]["LinkBomRevisionRequest"];
export type LinkDocRevisionRequest = components["schemas"]["LinkDocRevisionRequest"];
export type PaginatedResponse_BomHeader = components["schemas"]["PaginatedResponse_BomHeader"];
export type PaginatedResponse_BomLine = components["schemas"]["PaginatedResponse_BomLine"];
export type PaginatedResponse_BomRevision = components["schemas"]["PaginatedResponse_BomRevision"];
export type PaginatedResponse_Eco = components["schemas"]["PaginatedResponse_Eco"];
export type PaginatedResponse_EcoAuditEntry = components["schemas"]["PaginatedResponse_EcoAuditEntry"];
export type PaginatedResponse_EcoBomRevision = components["schemas"]["PaginatedResponse_EcoBomRevision"];
export type PaginatedResponse_EcoDocRevision = components["schemas"]["PaginatedResponse_EcoDocRevision"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type SetEffectivityRequest = components["schemas"]["SetEffectivityRequest"];
export type UpdateLineRequest = components["schemas"]["UpdateLineRequest"];
export type WhereUsedRow = components["schemas"]["WhereUsedRow"];

export interface BomClientOptions {
  baseUrl: string;
  token: string;
}

export function createBomClient(opts: BomClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
