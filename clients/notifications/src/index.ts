// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./notifications.d.ts";

export type { paths, components } from "./notifications.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type ApiError = components["schemas"]["ApiError"];
export type CreateTemplate = components["schemas"]["CreateTemplate"];
export type DeliveryAttemptDetail = components["schemas"]["DeliveryAttemptDetail"];
export type DeliveryListResponse = components["schemas"]["DeliveryListResponse"];
export type DeliveryQuery = components["schemas"]["DeliveryQuery"];
export type DeliveryReceipt = components["schemas"]["DeliveryReceipt"];
export type DlqActionResponse = components["schemas"]["DlqActionResponse"];
export type DlqDetailResponse = components["schemas"]["DlqDetailResponse"];
export type DlqError = components["schemas"]["DlqError"];
export type DlqItem = components["schemas"]["DlqItem"];
export type DlqListResponse = components["schemas"]["DlqListResponse"];
export type ErrorResponse = components["schemas"]["ErrorResponse"];
export type FieldError = components["schemas"]["FieldError"];
export type InboxActionResponse = components["schemas"]["InboxActionResponse"];
export type InboxError = components["schemas"]["InboxError"];
export type InboxItem = components["schemas"]["InboxItem"];
export type InboxListResponse = components["schemas"]["InboxListResponse"];
export type SendDetailResponse = components["schemas"]["SendDetailResponse"];
export type SendRequest = components["schemas"]["SendRequest"];
export type SendResponse = components["schemas"]["SendResponse"];
export type TemplateDetailResponse = components["schemas"]["TemplateDetailResponse"];
export type TemplateResponse = components["schemas"]["TemplateResponse"];
export type TemplateVersionSummary = components["schemas"]["TemplateVersionSummary"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type NotificationsClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createNotificationsClient(opts: NotificationsClientOptions) {
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
