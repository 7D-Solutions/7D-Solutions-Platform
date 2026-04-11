// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import type { paths, components } from "./shipping-receiving.d.ts";

export type { paths, components } from "./shipping-receiving.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type AddLineRequest = components["schemas"]["AddLineRequest"];
export type ApiError = components["schemas"]["ApiError"];
export type CreateShipmentRequest = components["schemas"]["CreateShipmentRequest"];
export type Direction = components["schemas"]["Direction"];
export type FieldError = components["schemas"]["FieldError"];
export type InspectionRoutingRow = components["schemas"]["InspectionRoutingRow"];
export type PaginatedResponse_Shipment = components["schemas"]["PaginatedResponse_Shipment"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type ReceiveLineRequest = components["schemas"]["ReceiveLineRequest"];
export type RouteLineRequest = components["schemas"]["RouteLineRequest"];
export type ShipLineQtyRequest = components["schemas"]["ShipLineQtyRequest"];
export type Shipment = components["schemas"]["Shipment"];
export type ShipmentLineRow = components["schemas"]["ShipmentLineRow"];
export type TransitionStatusRequest = components["schemas"]["TransitionStatusRequest"];

export interface ShippingReceivingClientOptions {
  baseUrl: string;
  token: string;
}

export function createShippingReceivingClient(opts: ShippingReceivingClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
