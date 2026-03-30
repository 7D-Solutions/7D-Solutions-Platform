import createClient from "openapi-fetch";
import type { paths, components } from "./bom.d.ts";

export type { paths, components } from "./bom.d.ts";

// Re-export common schema types for convenience
export type BomHeader = components["schemas"]["BomHeader"];
export type BomLine = components["schemas"]["BomLine"];
export type BomRevision = components["schemas"]["BomRevision"];
export type ExplosionRow = components["schemas"]["ExplosionRow"];
export type WhereUsedRow = components["schemas"]["WhereUsedRow"];
export type Eco = components["schemas"]["Eco"];
export type EcoAuditEntry = components["schemas"]["EcoAuditEntry"];
export type ApiError = components["schemas"]["ApiError"];

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
