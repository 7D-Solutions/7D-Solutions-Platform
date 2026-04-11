// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import type { paths, components } from "./maintenance.d.ts";

export type { paths, components } from "./maintenance.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AddLaborRequest = components["schemas"]["AddLaborRequest"];
export type AddPartRequest = components["schemas"]["AddPartRequest"];
export type ApiError = components["schemas"]["ApiError"];
export type Asset = components["schemas"]["Asset"];
export type AssetStatus = components["schemas"]["AssetStatus"];
export type AssetType = components["schemas"]["AssetType"];
export type AssignPlanRequest = components["schemas"]["AssignPlanRequest"];
export type CalibrationEvent = components["schemas"]["CalibrationEvent"];
export type CalibrationStatus = components["schemas"]["CalibrationStatus"];
export type CalibrationStatusResponse = components["schemas"]["CalibrationStatusResponse"];
export type CreateAssetRequest = components["schemas"]["CreateAssetRequest"];
export type CreateDowntimeRequest = components["schemas"]["CreateDowntimeRequest"];
export type CreateMeterTypeRequest = components["schemas"]["CreateMeterTypeRequest"];
export type CreatePlanRequest = components["schemas"]["CreatePlanRequest"];
export type CreateWorkOrderRequest = components["schemas"]["CreateWorkOrderRequest"];
export type DowntimeEvent = components["schemas"]["DowntimeEvent"];
export type FieldError = components["schemas"]["FieldError"];
export type MaintenancePlan = components["schemas"]["MaintenancePlan"];
export type MeterReading = components["schemas"]["MeterReading"];
export type MeterType = components["schemas"]["MeterType"];
export type PaginatedResponse_Asset = components["schemas"]["PaginatedResponse_Asset"];
export type PaginatedResponse_DowntimeEvent = components["schemas"]["PaginatedResponse_DowntimeEvent"];
export type PaginatedResponse_MaintenancePlan = components["schemas"]["PaginatedResponse_MaintenancePlan"];
export type PaginatedResponse_MeterReading = components["schemas"]["PaginatedResponse_MeterReading"];
export type PaginatedResponse_MeterType = components["schemas"]["PaginatedResponse_MeterType"];
export type PaginatedResponse_PlanAssignment = components["schemas"]["PaginatedResponse_PlanAssignment"];
export type PaginatedResponse_WoLabor = components["schemas"]["PaginatedResponse_WoLabor"];
export type PaginatedResponse_WoPart = components["schemas"]["PaginatedResponse_WoPart"];
export type PaginatedResponse_WorkOrder = components["schemas"]["PaginatedResponse_WorkOrder"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type PlanAssignment = components["schemas"]["PlanAssignment"];
export type Priority = components["schemas"]["Priority"];
export type RecordCalibrationRequest = components["schemas"]["RecordCalibrationRequest"];
export type RecordReadingRequest = components["schemas"]["RecordReadingRequest"];
export type ScheduleType = components["schemas"]["ScheduleType"];
export type TransitionRequest = components["schemas"]["TransitionRequest"];
export type UpdateAssetRequest = components["schemas"]["UpdateAssetRequest"];
export type UpdatePlanRequest = components["schemas"]["UpdatePlanRequest"];
export type WoLabor = components["schemas"]["WoLabor"];
export type WoPart = components["schemas"]["WoPart"];
export type WoStatus = components["schemas"]["WoStatus"];
export type WoType = components["schemas"]["WoType"];
export type WorkOrder = components["schemas"]["WorkOrder"];

export interface MaintenanceClientOptions {
  baseUrl: string;
  token: string;
}

export function createMaintenanceClient(opts: MaintenanceClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
