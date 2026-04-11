// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import type { paths, components } from "./inventory.d.ts";

export type { paths, components } from "./inventory.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type ActivateRevisionRequest = components["schemas"]["ActivateRevisionRequest"];
export type AdjustRequest = components["schemas"]["AdjustRequest"];
export type AdjustResult = components["schemas"]["AdjustResult"];
export type ApiError = components["schemas"]["ApiError"];
export type ApproveBody = components["schemas"]["ApproveBody"];
export type ConsumedLayer = components["schemas"]["ConsumedLayer"];
export type CreateConversionRequest = components["schemas"]["CreateConversionRequest"];
export type CreateItemRequest = components["schemas"]["CreateItemRequest"];
export type CreateLocationRequest = components["schemas"]["CreateLocationRequest"];
export type CreateReorderPolicyRequest = components["schemas"]["CreateReorderPolicyRequest"];
export type CreateRevisionRequest = components["schemas"]["CreateRevisionRequest"];
export type CreateSnapshotRequest = components["schemas"]["CreateSnapshotRequest"];
export type CreateTaskRequest = components["schemas"]["CreateTaskRequest"];
export type CreateTaskResult = components["schemas"]["CreateTaskResult"];
export type CreateUomRequest = components["schemas"]["CreateUomRequest"];
export type FieldError = components["schemas"]["FieldError"];
export type FulfillRequest = components["schemas"]["FulfillRequest"];
export type FulfillResult = components["schemas"]["FulfillResult"];
export type GenealogyEdge = components["schemas"]["GenealogyEdge"];
export type GenealogyResult = components["schemas"]["GenealogyResult"];
export type GenerateLabelRequest = components["schemas"]["GenerateLabelRequest"];
export type InvItemStatus = components["schemas"]["InvItemStatus"];
export type InventoryLot = components["schemas"]["InventoryLot"];
export type IssueRequest = components["schemas"]["IssueRequest"];
export type IssueResult = components["schemas"]["IssueResult"];
export type Item = components["schemas"]["Item"];
export type ItemRevision = components["schemas"]["ItemRevision"];
export type ItemUomConversion = components["schemas"]["ItemUomConversion"];
export type Label = components["schemas"]["Label"];
export type ListItemsQuery = components["schemas"]["ListItemsQuery"];
export type Location = components["schemas"]["Location"];
export type LotExpiryRecord = components["schemas"]["LotExpiryRecord"];
export type LotMergeRequest = components["schemas"]["LotMergeRequest"];
export type LotSplitRequest = components["schemas"]["LotSplitRequest"];
export type MergeParent = components["schemas"]["MergeParent"];
export type MovementEntry = components["schemas"]["MovementEntry"];
export type PaginatedResponse_InventoryLot = components["schemas"]["PaginatedResponse_InventoryLot"];
export type PaginatedResponse_Item = components["schemas"]["PaginatedResponse_Item"];
export type PaginatedResponse_ItemRevision = components["schemas"]["PaginatedResponse_ItemRevision"];
export type PaginatedResponse_Label = components["schemas"]["PaginatedResponse_Label"];
export type PaginatedResponse_Location = components["schemas"]["PaginatedResponse_Location"];
export type PaginatedResponse_ReorderPolicy = components["schemas"]["PaginatedResponse_ReorderPolicy"];
export type PaginatedResponse_ValuationSnapshot = components["schemas"]["PaginatedResponse_ValuationSnapshot"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type ReceiptRequest = components["schemas"]["ReceiptRequest"];
export type ReceiptResult = components["schemas"]["ReceiptResult"];
export type ReleaseRequest = components["schemas"]["ReleaseRequest"];
export type ReleaseResult = components["schemas"]["ReleaseResult"];
export type ReorderPolicy = components["schemas"]["ReorderPolicy"];
export type ReserveRequest = components["schemas"]["ReserveRequest"];
export type ReserveResult = components["schemas"]["ReserveResult"];
export type RunExpiryAlertScanRequest = components["schemas"]["RunExpiryAlertScanRequest"];
export type RunExpiryAlertScanResult = components["schemas"]["RunExpiryAlertScanResult"];
export type SetLotExpiryRequest = components["schemas"]["SetLotExpiryRequest"];
export type SetMakeBuyRequest = components["schemas"]["SetMakeBuyRequest"];
export type SourceRef = components["schemas"]["SourceRef"];
export type SplitChild = components["schemas"]["SplitChild"];
export type StatusTransferRequest = components["schemas"]["StatusTransferRequest"];
export type StatusTransferResult = components["schemas"]["StatusTransferResult"];
export type SubmitBody = components["schemas"]["SubmitBody"];
export type SubmitLineInput = components["schemas"]["SubmitLineInput"];
export type TaskLine = components["schemas"]["TaskLine"];
export type TaskScope = components["schemas"]["TaskScope"];
export type TrackingMode = components["schemas"]["TrackingMode"];
export type TransferRequest = components["schemas"]["TransferRequest"];
export type TransferResult = components["schemas"]["TransferResult"];
export type Uom = components["schemas"]["Uom"];
export type UpdateItemRequest = components["schemas"]["UpdateItemRequest"];
export type UpdateLocationRequest = components["schemas"]["UpdateLocationRequest"];
export type UpdateReorderPolicyRequest = components["schemas"]["UpdateReorderPolicyRequest"];
export type UpdateRevisionPolicyRequest = components["schemas"]["UpdateRevisionPolicyRequest"];
export type ValuationLine = components["schemas"]["ValuationLine"];
export type ValuationSnapshot = components["schemas"]["ValuationSnapshot"];

export interface InventoryClientOptions {
  baseUrl: string;
  token: string;
}

export function createInventoryClient(opts: InventoryClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
