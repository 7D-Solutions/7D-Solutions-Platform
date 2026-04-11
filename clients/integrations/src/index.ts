// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import type { paths, components } from "./integrations.d.ts";

export type { paths, components } from "./integrations.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type ApiError = components["schemas"]["ApiError"];
export type ConfigField = components["schemas"]["ConfigField"];
export type ConfigFieldType = components["schemas"]["ConfigFieldType"];
export type ConnectionStatus = components["schemas"]["ConnectionStatus"];
export type ConnectorCapabilities = components["schemas"]["ConnectorCapabilities"];
export type ConnectorConfig = components["schemas"]["ConnectorConfig"];
export type CreateExternalRefRequest = components["schemas"]["CreateExternalRefRequest"];
export type ExternalRef = components["schemas"]["ExternalRef"];
export type FieldError = components["schemas"]["FieldError"];
export type OAuthConnectionInfo = components["schemas"]["OAuthConnectionInfo"];
export type PaginatedResponse_ConnectorCapabilities = components["schemas"]["PaginatedResponse_ConnectorCapabilities"];
export type PaginatedResponse_ConnectorConfig = components["schemas"]["PaginatedResponse_ConnectorConfig"];
export type PaginatedResponse_ExternalRef = components["schemas"]["PaginatedResponse_ExternalRef"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type RegisterConnectorRequest = components["schemas"]["RegisterConnectorRequest"];
export type RunTestActionRequest = components["schemas"]["RunTestActionRequest"];
export type TestActionResult = components["schemas"]["TestActionResult"];
export type UpdateExternalRefRequest = components["schemas"]["UpdateExternalRefRequest"];
export type UpdateInvoiceRequest = components["schemas"]["UpdateInvoiceRequest"];
export type UpdateInvoiceResponse = components["schemas"]["UpdateInvoiceResponse"];

export interface IntegrationsClientOptions {
  baseUrl: string;
  token: string;
}

export function createIntegrationsClient(opts: IntegrationsClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
