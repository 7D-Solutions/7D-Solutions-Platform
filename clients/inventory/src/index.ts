import createClient from "openapi-fetch";
import type { paths, components } from "./inventory.d.ts";

export type { paths, components } from "./inventory.d.ts";

// Re-export common schema types for convenience
export type Item = components["schemas"]["Item"];
export type CreateItemRequest = components["schemas"]["CreateItemRequest"];
export type UpdateItemRequest = components["schemas"]["UpdateItemRequest"];
export type Location = components["schemas"]["Location"];
export type CreateLocationRequest = components["schemas"]["CreateLocationRequest"];
export type UpdateLocationRequest = components["schemas"]["UpdateLocationRequest"];
export type ReceiptRequest = components["schemas"]["ReceiptRequest"];
export type ReceiptResult = components["schemas"]["ReceiptResult"];
export type IssueRequest = components["schemas"]["IssueRequest"];
export type IssueResult = components["schemas"]["IssueResult"];
export type TransferRequest = components["schemas"]["TransferRequest"];
export type TransferResult = components["schemas"]["TransferResult"];
export type AdjustRequest = components["schemas"]["AdjustRequest"];
export type AdjustResult = components["schemas"]["AdjustResult"];
export type ReserveRequest = components["schemas"]["ReserveRequest"];
export type ReserveResult = components["schemas"]["ReserveResult"];
export type ReleaseRequest = components["schemas"]["ReleaseRequest"];
export type ReleaseResult = components["schemas"]["ReleaseResult"];
export type FulfillRequest = components["schemas"]["FulfillRequest"];
export type FulfillResult = components["schemas"]["FulfillResult"];
export type Uom = components["schemas"]["Uom"];
export type ItemUomConversion = components["schemas"]["ItemUomConversion"];
export type ReorderPolicy = components["schemas"]["ReorderPolicy"];
export type ValuationSnapshot = components["schemas"]["ValuationSnapshot"];
export type ItemRevision = components["schemas"]["ItemRevision"];
export type Label = components["schemas"]["Label"];
export type InventoryLot = components["schemas"]["InventoryLot"];
export type MovementEntry = components["schemas"]["MovementEntry"];
export type GenealogyEdge = components["schemas"]["GenealogyEdge"];
export type GenealogyResult = components["schemas"]["GenealogyResult"];
export type ApiError = components["schemas"]["ApiError"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];

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
