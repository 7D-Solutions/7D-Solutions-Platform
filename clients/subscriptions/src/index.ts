// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./subscriptions.d.ts";

export type { paths, components } from "./subscriptions.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type ApiError = components["schemas"]["ApiError"];
export type BillRun = components["schemas"]["BillRun"];
export type BillRunResult = components["schemas"]["BillRunResult"];
export type CancelSubscriptionRequest = components["schemas"]["CancelSubscriptionRequest"];
export type CreateSubscriptionPlanRequest = components["schemas"]["CreateSubscriptionPlanRequest"];
export type CreateSubscriptionRequest = components["schemas"]["CreateSubscriptionRequest"];
export type ExecuteBillRunRequest = components["schemas"]["ExecuteBillRunRequest"];
export type FieldError = components["schemas"]["FieldError"];
export type PauseSubscriptionRequest = components["schemas"]["PauseSubscriptionRequest"];
export type Subscription = components["schemas"]["Subscription"];
export type SubscriptionPlan = components["schemas"]["SubscriptionPlan"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type SubscriptionsClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createSubscriptionsClient(opts: SubscriptionsClientOptions) {
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
