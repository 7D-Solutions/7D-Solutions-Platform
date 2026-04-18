// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./payments.d.ts";

export type { paths, components } from "./payments.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type ApiError = components["schemas"]["ApiError"];
export type CheckoutSessionStatusResponse = components["schemas"]["CheckoutSessionStatusResponse"];
export type ConsistencyCheckSchema = components["schemas"]["ConsistencyCheckSchema"];
export type CreateCheckoutSessionRequest = components["schemas"]["CreateCheckoutSessionRequest"];
export type CreateCheckoutSessionResponse = components["schemas"]["CreateCheckoutSessionResponse"];
export type CursorStatusSchema = components["schemas"]["CursorStatusSchema"];
export type DataSource = components["schemas"]["DataSource"];
export type FieldError = components["schemas"]["FieldError"];
export type PaginatedResponse_ProjectionSummarySchema = components["schemas"]["PaginatedResponse_ProjectionSummarySchema"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type PaymentResponse = components["schemas"]["PaymentResponse"];
export type ProjectionStatusSchema = components["schemas"]["ProjectionStatusSchema"];
export type ProjectionSummarySchema = components["schemas"]["ProjectionSummarySchema"];
export type SessionStatusPollResponse = components["schemas"]["SessionStatusPollResponse"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type PaymentsClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createPaymentsClient(opts: PaymentsClientOptions) {
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
