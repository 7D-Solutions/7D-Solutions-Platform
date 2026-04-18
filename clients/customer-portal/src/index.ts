// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./customer-portal.d.ts";

export type { paths, components } from "./customer-portal.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AcknowledgeRequest = components["schemas"]["AcknowledgeRequest"];
export type ApiError = components["schemas"]["ApiError"];
export type AuthResponse = components["schemas"]["AuthResponse"];
export type CreateStatusCardRequest = components["schemas"]["CreateStatusCardRequest"];
export type FieldError = components["schemas"]["FieldError"];
export type InviteUserRequest = components["schemas"]["InviteUserRequest"];
export type InviteUserResponse = components["schemas"]["InviteUserResponse"];
export type LinkDocumentRequest = components["schemas"]["LinkDocumentRequest"];
export type LoginRequest = components["schemas"]["LoginRequest"];
export type LogoutRequest = components["schemas"]["LogoutRequest"];
export type MeResponse = components["schemas"]["MeResponse"];
export type PaginatedResponse_PortalDocumentView = components["schemas"]["PaginatedResponse_PortalDocumentView"];
export type PaginatedResponse_StatusCard = components["schemas"]["PaginatedResponse_StatusCard"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type PortalDocumentView = components["schemas"]["PortalDocumentView"];
export type RefreshRequest = components["schemas"]["RefreshRequest"];
export type StatusCard = components["schemas"]["StatusCard"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type CustomerPortalClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createCustomerPortalClient(opts: CustomerPortalClientOptions) {
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
