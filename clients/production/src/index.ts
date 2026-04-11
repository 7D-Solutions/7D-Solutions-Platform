// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import type { paths, components } from "./production.d.ts";

export type { paths, components } from "./production.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AddRoutingStepRequest = components["schemas"]["AddRoutingStepRequest"];
export type ApiError = components["schemas"]["ApiError"];
export type ComponentIssueItemInput = components["schemas"]["ComponentIssueItemInput"];
export type CreateRoutingRequest = components["schemas"]["CreateRoutingRequest"];
export type CreateWorkOrderRequest = components["schemas"]["CreateWorkOrderRequest"];
export type CreateWorkcenterRequest = components["schemas"]["CreateWorkcenterRequest"];
export type EndDowntimeRequest = components["schemas"]["EndDowntimeRequest"];
export type FieldError = components["schemas"]["FieldError"];
export type ItemDateQuery = components["schemas"]["ItemDateQuery"];
export type ManualEntryRequest = components["schemas"]["ManualEntryRequest"];
export type OperationInstance = components["schemas"]["OperationInstance"];
export type PaginatedResponse_OperationInstance = components["schemas"]["PaginatedResponse_OperationInstance"];
export type PaginatedResponse_RoutingStep = components["schemas"]["PaginatedResponse_RoutingStep"];
export type PaginatedResponse_RoutingTemplate = components["schemas"]["PaginatedResponse_RoutingTemplate"];
export type PaginatedResponse_TimeEntry = components["schemas"]["PaginatedResponse_TimeEntry"];
export type PaginatedResponse_Workcenter = components["schemas"]["PaginatedResponse_Workcenter"];
export type PaginatedResponse_WorkcenterDowntime = components["schemas"]["PaginatedResponse_WorkcenterDowntime"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type PaginationQuery = components["schemas"]["PaginationQuery"];
export type RequestComponentIssueRequest = components["schemas"]["RequestComponentIssueRequest"];
export type RequestFgReceiptRequest = components["schemas"]["RequestFgReceiptRequest"];
export type RoutingStep = components["schemas"]["RoutingStep"];
export type RoutingTemplate = components["schemas"]["RoutingTemplate"];
export type StartDowntimeRequest = components["schemas"]["StartDowntimeRequest"];
export type StartTimerRequest = components["schemas"]["StartTimerRequest"];
export type StopTimerRequest = components["schemas"]["StopTimerRequest"];
export type TimeEntry = components["schemas"]["TimeEntry"];
export type UpdateRoutingRequest = components["schemas"]["UpdateRoutingRequest"];
export type UpdateWorkcenterRequest = components["schemas"]["UpdateWorkcenterRequest"];
export type WorkOrder = components["schemas"]["WorkOrder"];
export type WorkOrderStatus = components["schemas"]["WorkOrderStatus"];
export type Workcenter = components["schemas"]["Workcenter"];
export type WorkcenterDowntime = components["schemas"]["WorkcenterDowntime"];

export interface ProductionClientOptions {
  baseUrl: string;
  token: string;
}

export function createProductionClient(opts: ProductionClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
