// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./numbering.d.ts";

export type { paths, components } from "./numbering.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AllocateRequest = components["schemas"]["AllocateRequest"];
export type AllocateResponse = components["schemas"]["AllocateResponse"];
export type ApiError = components["schemas"]["ApiError"];
export type ConfirmRequest = components["schemas"]["ConfirmRequest"];
export type ConfirmResponse = components["schemas"]["ConfirmResponse"];
export type FieldError = components["schemas"]["FieldError"];
export type PolicyResponse = components["schemas"]["PolicyResponse"];
export type UpsertPolicyRequest = components["schemas"]["UpsertPolicyRequest"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type NumberingClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createNumberingClient(opts: NumberingClientOptions) {
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
