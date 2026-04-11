// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
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

export interface NumberingClientOptions {
  baseUrl: string;
  token: string;
}

export function createNumberingClient(opts: NumberingClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
