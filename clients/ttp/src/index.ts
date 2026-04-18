// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./ttp.d.ts";

export type { paths, components } from "./ttp.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type ApiError = components["schemas"]["ApiError"];
export type BillingRunRequest = components["schemas"]["BillingRunRequest"];
export type BillingRunResponse = components["schemas"]["BillingRunResponse"];
export type EventItem = components["schemas"]["EventItem"];
export type FieldError = components["schemas"]["FieldError"];
export type IngestEventRequest = components["schemas"]["IngestEventRequest"];
export type IngestEventResponse = components["schemas"]["IngestEventResponse"];
export type IngestResultItem = components["schemas"]["IngestResultItem"];
export type ListServiceAgreementsResponse = components["schemas"]["ListServiceAgreementsResponse"];
export type PriceTrace = components["schemas"]["PriceTrace"];
export type ServiceAgreementItem = components["schemas"]["ServiceAgreementItem"];
export type TraceLineItem = components["schemas"]["TraceLineItem"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type TtpClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createTtpClient(opts: TtpClientOptions) {
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
