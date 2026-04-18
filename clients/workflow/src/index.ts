// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./workflow.d.ts";

export type { paths, components } from "./workflow.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AdvanceInstanceRequest = components["schemas"]["AdvanceInstanceRequest"];
export type AdvanceResponse = components["schemas"]["AdvanceResponse"];
export type ApiError = components["schemas"]["ApiError"];
export type CreateDefinitionRequest = components["schemas"]["CreateDefinitionRequest"];
export type FieldError = components["schemas"]["FieldError"];
export type InstanceStatus = components["schemas"]["InstanceStatus"];
export type PaginatedResponse_WorkflowDefinition = components["schemas"]["PaginatedResponse_WorkflowDefinition"];
export type PaginatedResponse_WorkflowInstance = components["schemas"]["PaginatedResponse_WorkflowInstance"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type StartInstanceRequest = components["schemas"]["StartInstanceRequest"];
export type WorkflowDefinition = components["schemas"]["WorkflowDefinition"];
export type WorkflowInstance = components["schemas"]["WorkflowInstance"];
export type WorkflowTransition = components["schemas"]["WorkflowTransition"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type WorkflowClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createWorkflowClient(opts: WorkflowClientOptions) {
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
