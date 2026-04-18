// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import { createAuthMiddleware } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";
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

export type { AuthClient } from "@7d/auth-client";
export { createAuthMiddleware } from "@7d/auth-client";

export type ShippingReceivingClientOptions =
  | { baseUrl: string; token: string }
  | { baseUrl: string; authClient: AuthClient };

export function createShippingReceivingClient(opts: ShippingReceivingClientOptions) {
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
