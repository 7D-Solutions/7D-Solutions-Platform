// @generated — do not edit by hand. Re-run ts-codegen.mjs to regenerate.
import createClient from "openapi-fetch";
import type { paths, components } from "./party.d.ts";

export type { paths, components } from "./party.d.ts";

// ── Schema type re-exports ──────────────────────────────────────
export type Address = components["schemas"]["Address"];
export type ApiError = components["schemas"]["ApiError"];
export type Contact = components["schemas"]["Contact"];
export type CreateAddressRequest = components["schemas"]["CreateAddressRequest"];
export type CreateCompanyRequest = components["schemas"]["CreateCompanyRequest"];
export type CreateContactRequest = components["schemas"]["CreateContactRequest"];
export type CreateIndividualRequest = components["schemas"]["CreateIndividualRequest"];
export type DataResponse_Address = components["schemas"]["DataResponse_Address"];
export type DataResponse_Contact = components["schemas"]["DataResponse_Contact"];
export type DataResponse_PrimaryContactEntry = components["schemas"]["DataResponse_PrimaryContactEntry"];
export type ExternalRef = components["schemas"]["ExternalRef"];
export type FieldError = components["schemas"]["FieldError"];
export type ListPartiesQuery = components["schemas"]["ListPartiesQuery"];
export type PaginatedResponse_Party = components["schemas"]["PaginatedResponse_Party"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type Party = components["schemas"]["Party"];
export type PartyCompany = components["schemas"]["PartyCompany"];
export type PartyIndividual = components["schemas"]["PartyIndividual"];
export type PartyView = components["schemas"]["PartyView"];
export type PrimaryContactEntry = components["schemas"]["PrimaryContactEntry"];
export type SearchQuery = components["schemas"]["SearchQuery"];
export type SetPrimaryRequest = components["schemas"]["SetPrimaryRequest"];
export type UpdateAddressRequest = components["schemas"]["UpdateAddressRequest"];
export type UpdateContactRequest = components["schemas"]["UpdateContactRequest"];
export type UpdatePartyRequest = components["schemas"]["UpdatePartyRequest"];

export interface PartyClientOptions {
  baseUrl: string;
  token: string;
}

export function createPartyClient(opts: PartyClientOptions) {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    headers: {
      Authorization: `Bearer ${opts.token}`,
      "Content-Type": "application/json",
    },
  });
}
