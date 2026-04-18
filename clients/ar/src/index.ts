// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./ar.d.ts";

export type { paths, components } from "./ar.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AddPaymentMethodRequest = components["schemas"]["AddPaymentMethodRequest"];
export type ApiError = components["schemas"]["ApiError"];
export type CancelSubscriptionRequest = components["schemas"]["CancelSubscriptionRequest"];
export type CaptureChargeRequest = components["schemas"]["CaptureChargeRequest"];
export type Charge = components["schemas"]["Charge"];
export type CreateChargeRequest = components["schemas"]["CreateChargeRequest"];
export type CreateCustomerRequest = components["schemas"]["CreateCustomerRequest"];
export type CreateInvoiceRequest = components["schemas"]["CreateInvoiceRequest"];
export type CreateRefundRequest = components["schemas"]["CreateRefundRequest"];
export type CreateSubscriptionRequest = components["schemas"]["CreateSubscriptionRequest"];
export type Customer = components["schemas"]["Customer"];
export type Dispute = components["schemas"]["Dispute"];
export type ErrorResponse = components["schemas"]["ErrorResponse"];
export type Event = components["schemas"]["Event"];
export type FieldError = components["schemas"]["FieldError"];
export type FinalizeInvoiceRequest = components["schemas"]["FinalizeInvoiceRequest"];
export type Invoice = components["schemas"]["Invoice"];
export type InvoiceLineItem = components["schemas"]["InvoiceLineItem"];
export type PaginatedResponse_Charge = components["schemas"]["PaginatedResponse_Charge"];
export type PaginatedResponse_Customer = components["schemas"]["PaginatedResponse_Customer"];
export type PaginatedResponse_Dispute = components["schemas"]["PaginatedResponse_Dispute"];
export type PaginatedResponse_Event = components["schemas"]["PaginatedResponse_Event"];
export type PaginatedResponse_Invoice = components["schemas"]["PaginatedResponse_Invoice"];
export type PaginatedResponse_PaymentMethod = components["schemas"]["PaginatedResponse_PaymentMethod"];
export type PaginatedResponse_Refund = components["schemas"]["PaginatedResponse_Refund"];
export type PaginatedResponse_ScheduledRunExecutionOutcome = components["schemas"]["PaginatedResponse_ScheduledRunExecutionOutcome"];
export type PaginatedResponse_Subscription = components["schemas"]["PaginatedResponse_Subscription"];
export type PaginatedResponse_Webhook = components["schemas"]["PaginatedResponse_Webhook"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type PaymentMethod = components["schemas"]["PaymentMethod"];
export type Refund = components["schemas"]["Refund"];
export type ScheduledRunExecutionOutcome = components["schemas"]["ScheduledRunExecutionOutcome"];
export type ScheduledRunResult = components["schemas"]["ScheduledRunResult"];
export type SubmitDisputeEvidenceRequest = components["schemas"]["SubmitDisputeEvidenceRequest"];
export type Subscription = components["schemas"]["Subscription"];
export type SubscriptionInterval = components["schemas"]["SubscriptionInterval"];
export type SubscriptionStatus = components["schemas"]["SubscriptionStatus"];
export type UpdateCustomerRequest = components["schemas"]["UpdateCustomerRequest"];
export type UpdateInvoiceRequest = components["schemas"]["UpdateInvoiceRequest"];
export type UpdatePaymentMethodRequest = components["schemas"]["UpdatePaymentMethodRequest"];
export type UpdateSubscriptionRequest = components["schemas"]["UpdateSubscriptionRequest"];
export type Value = components["schemas"]["Value"];
export type Webhook = components["schemas"]["Webhook"];
export type WebhookStatus = components["schemas"]["WebhookStatus"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type ArClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createArClient(opts: ArClientOptions) {
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
