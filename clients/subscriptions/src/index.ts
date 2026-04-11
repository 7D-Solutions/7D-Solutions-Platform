// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
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

export interface SubscriptionsClientOptions {
  baseUrl: string;
  token: string;
}

export function createSubscriptionsClient(opts: SubscriptionsClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
