// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./pdf-editor.d.ts";

export type { paths, components } from "./pdf-editor.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type ApiError = components["schemas"]["ApiError"];
export type AutosaveRequest = components["schemas"]["AutosaveRequest"];
export type CreateFieldRequest = components["schemas"]["CreateFieldRequest"];
export type CreateSubmissionRequest = components["schemas"]["CreateSubmissionRequest"];
export type CreateTemplateRequest = components["schemas"]["CreateTemplateRequest"];
export type FieldError = components["schemas"]["FieldError"];
export type FormField = components["schemas"]["FormField"];
export type FormSubmission = components["schemas"]["FormSubmission"];
export type FormTemplate = components["schemas"]["FormTemplate"];
export type PaginatedResponse_FormSubmission = components["schemas"]["PaginatedResponse_FormSubmission"];
export type PaginatedResponse_FormTemplate = components["schemas"]["PaginatedResponse_FormTemplate"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type ReorderFieldsRequest = components["schemas"]["ReorderFieldsRequest"];
export type UpdateFieldRequest = components["schemas"]["UpdateFieldRequest"];
export type UpdateTemplateRequest = components["schemas"]["UpdateTemplateRequest"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type PdfEditorClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createPdfEditorClient(opts: PdfEditorClientOptions) {
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
