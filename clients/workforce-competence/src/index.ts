// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import type { paths, components } from "./workforce-competence.d.ts";

export type { paths, components } from "./workforce-competence.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AcceptanceAuthority = components["schemas"]["AcceptanceAuthority"];
export type AcceptanceAuthorityResult = components["schemas"]["AcceptanceAuthorityResult"];
export type ApiError = components["schemas"]["ApiError"];
export type ArtifactType = components["schemas"]["ArtifactType"];
export type AssignCompetenceRequest = components["schemas"]["AssignCompetenceRequest"];
export type AuthorizationResult = components["schemas"]["AuthorizationResult"];
export type CompetenceArtifact = components["schemas"]["CompetenceArtifact"];
export type FieldError = components["schemas"]["FieldError"];
export type GrantAuthorityRequest = components["schemas"]["GrantAuthorityRequest"];
export type OperatorCompetence = components["schemas"]["OperatorCompetence"];
export type RegisterArtifactRequest = components["schemas"]["RegisterArtifactRequest"];
export type RevokeAuthorityRequest = components["schemas"]["RevokeAuthorityRequest"];

export interface WorkforceCompetenceClientOptions {
  baseUrl: string;
  token: string;
}

export function createWorkforceCompetenceClient(opts: WorkforceCompetenceClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
