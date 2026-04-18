// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
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

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type WorkforceCompetenceClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createWorkforceCompetenceClient(opts: WorkforceCompetenceClientOptions) {
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
