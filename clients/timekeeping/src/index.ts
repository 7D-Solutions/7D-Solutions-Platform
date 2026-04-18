// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
import type { paths, components } from "./timekeeping.d.ts";

export type { paths, components } from "./timekeeping.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type Allocation = components["schemas"]["Allocation"];
export type ApiError = components["schemas"]["ApiError"];
export type ApprovalAction = components["schemas"]["ApprovalAction"];
export type ApprovalRequest = components["schemas"]["ApprovalRequest"];
export type ApprovalStatus = components["schemas"]["ApprovalStatus"];
export type BillingLineItem = components["schemas"]["BillingLineItem"];
export type BillingRate = components["schemas"]["BillingRate"];
export type BillingRun = components["schemas"]["BillingRun"];
export type BillingRunResult = components["schemas"]["BillingRunResult"];
export type CorrectEntryRequest = components["schemas"]["CorrectEntryRequest"];
export type CreateAllocationRequest = components["schemas"]["CreateAllocationRequest"];
export type CreateBillingRateRequest = components["schemas"]["CreateBillingRateRequest"];
export type CreateBillingRunRequest = components["schemas"]["CreateBillingRunRequest"];
export type CreateEmployeeRequest = components["schemas"]["CreateEmployeeRequest"];
export type CreateEntryRequest = components["schemas"]["CreateEntryRequest"];
export type CreateExportRunRequest = components["schemas"]["CreateExportRunRequest"];
export type CreateProjectRequest = components["schemas"]["CreateProjectRequest"];
export type CreateTaskRequest = components["schemas"]["CreateTaskRequest"];
export type Employee = components["schemas"]["Employee"];
export type EmployeeRollup = components["schemas"]["EmployeeRollup"];
export type EntryType = components["schemas"]["EntryType"];
export type ExportArtifact = components["schemas"]["ExportArtifact"];
export type ExportRun = components["schemas"]["ExportRun"];
export type ExportStatus = components["schemas"]["ExportStatus"];
export type FieldError = components["schemas"]["FieldError"];
export type PaginatedResponse_Task = components["schemas"]["PaginatedResponse_Task"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type Project = components["schemas"]["Project"];
export type ProjectRollup = components["schemas"]["ProjectRollup"];
export type RecallApprovalRequest = components["schemas"]["RecallApprovalRequest"];
export type ReviewApprovalRequest = components["schemas"]["ReviewApprovalRequest"];
export type SubmitApprovalRequest = components["schemas"]["SubmitApprovalRequest"];
export type Task = components["schemas"]["Task"];
export type TaskRollup = components["schemas"]["TaskRollup"];
export type TimesheetEntry = components["schemas"]["TimesheetEntry"];
export type UpdateAllocationRequest = components["schemas"]["UpdateAllocationRequest"];
export type UpdateEmployeeRequest = components["schemas"]["UpdateEmployeeRequest"];
export type UpdateProjectRequest = components["schemas"]["UpdateProjectRequest"];
export type UpdateTaskRequest = components["schemas"]["UpdateTaskRequest"];
export type VoidEntryRequest = components["schemas"]["VoidEntryRequest"];

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type TimekeepingClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createTimekeepingClient(opts: TimekeepingClientOptions) {
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
