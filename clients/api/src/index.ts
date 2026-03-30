// ── Per-module re-exports ──────────────────────────────────────────
// Each module's factory function, typed paths, components, and
// convenience schema types are available as direct imports.

export {
  createInventoryClient,
  type InventoryClientOptions,
  type paths as InventoryPaths,
  type components as InventoryComponents,
  type Item,
  type CreateItemRequest,
  type UpdateItemRequest,
  type Location,
  type CreateLocationRequest,
  type UpdateLocationRequest,
  type ReceiptRequest,
  type ReceiptResult,
  type IssueRequest,
  type IssueResult,
  type TransferRequest,
  type TransferResult,
  type AdjustRequest,
  type AdjustResult,
  type ReserveRequest,
  type ReserveResult,
  type ReleaseRequest,
  type ReleaseResult,
  type FulfillRequest,
  type FulfillResult,
  type Uom,
  type ItemUomConversion,
  type ReorderPolicy,
  type ValuationSnapshot,
  type ItemRevision,
  type Label,
  type InventoryLot,
  type MovementEntry,
  type GenealogyEdge,
  type GenealogyResult,
} from "@7d/inventory-client";

export {
  createBomClient,
  type BomClientOptions,
  type paths as BomPaths,
  type components as BomComponents,
  type BomHeader,
  type BomLine,
  type BomRevision,
  type ExplosionRow,
  type WhereUsedRow,
  type Eco,
  type EcoAuditEntry,
} from "@7d/bom-client";

export {
  createPartyClient,
  type PartyClientOptions,
  type paths as PartyPaths,
  type components as PartyComponents,
  type Party,
  type PartyView,
  type PartyCompany,
  type PartyIndividual,
  type ExternalRef,
  type CreateCompanyRequest,
  type CreateIndividualRequest,
  type UpdatePartyRequest,
  type ListPartiesQuery,
  type SearchQuery,
  type Contact,
  type CreateContactRequest,
  type UpdateContactRequest,
  type SetPrimaryRequest,
  type PrimaryContactEntry,
  type Address,
  type CreateAddressRequest,
  type UpdateAddressRequest,
} from "@7d/party-client";

// ── Shared types ───────────────────────────────────────────────────
// Types that appear in multiple modules are re-exported once under
// a canonical name.  Per-module versions remain accessible via
// InventoryComponents["schemas"]["ApiError"], etc.

export type { ApiError, PaginationMeta } from "@7d/inventory-client";

// ── Unified client factory ─────────────────────────────────────────

import type { Client } from "openapi-fetch";
import { createInventoryClient } from "@7d/inventory-client";
import type { paths as InvPaths } from "@7d/inventory-client";
import { createBomClient } from "@7d/bom-client";
import type { paths as BomPathsImport } from "@7d/bom-client";
import { createPartyClient } from "@7d/party-client";
import type { paths as PartyPathsImport } from "@7d/party-client";

export interface ApiClientOptions {
  /** Bearer JWT for authentication. */
  token: string;
  /** Base URL shared by all services (e.g. API gateway). */
  baseUrl: string;
}

export interface ApiClient {
  inventory: Client<InvPaths>;
  bom: Client<BomPathsImport>;
  party: Client<PartyPathsImport>;
}

/**
 * Create a unified client for all 7D platform services.
 *
 * Uses a single base URL — suitable when services are behind an API
 * gateway.  For per-service URLs, use the individual factory functions
 * (createInventoryClient, createBomClient, createPartyClient) instead.
 */
export function createClient(opts: ApiClientOptions): ApiClient {
  return {
    inventory: createInventoryClient(opts),
    bom: createBomClient(opts),
    party: createPartyClient(opts),
  };
}
