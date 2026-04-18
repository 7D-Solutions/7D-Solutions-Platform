// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./platform-client-doc-mgmt.d.ts";

export type { paths, components } from "./platform-client-doc-mgmt.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type Document = components["schemas"]["Document"];
export type Revision = components["schemas"]["Revision"];
export type CreateDocumentRequest = components["schemas"]["CreateDocumentRequest"];
export type CreateRevisionRequest = components["schemas"]["CreateRevisionRequest"];
export type SupersedeRequest = components["schemas"]["SupersedeRequest"];
export type DocumentListResponse = components["schemas"]["DocumentListResponse"];
export type DocumentWithRevisionResponse = components["schemas"]["DocumentWithRevisionResponse"];
export type ReleaseResponse = components["schemas"]["ReleaseResponse"];
export type SupersedeResponse = components["schemas"]["SupersedeResponse"];
export type RevisionResponse = components["schemas"]["RevisionResponse"];
export type CreateDistributionRequest = components["schemas"]["CreateDistributionRequest"];
export type DistributionStatusUpdateRequest = components["schemas"]["DistributionStatusUpdateRequest"];
export type DocumentDistribution = components["schemas"]["DocumentDistribution"];
export type DistributionListResponse = components["schemas"]["DistributionListResponse"];
export type DistributionResponse = components["schemas"]["DistributionResponse"];
export type RetentionPolicy = components["schemas"]["RetentionPolicy"];
export type SetRetentionPolicyRequest = components["schemas"]["SetRetentionPolicyRequest"];
export type LegalHold = components["schemas"]["LegalHold"];
export type HoldListResponse = components["schemas"]["HoldListResponse"];
export type ApplyHoldRequest = components["schemas"]["ApplyHoldRequest"];
export type ReleaseHoldRequest = components["schemas"]["ReleaseHoldRequest"];
export type DocTemplate = components["schemas"]["DocTemplate"];
export type CreateTemplateRequest = components["schemas"]["CreateTemplateRequest"];
export type RenderRequest = components["schemas"]["RenderRequest"];
export type RenderArtifact = components["schemas"]["RenderArtifact"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type PlatformClientDocMgmtClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createPlatformClientDocMgmtClient(opts: PlatformClientDocMgmtClientOptions) {
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
