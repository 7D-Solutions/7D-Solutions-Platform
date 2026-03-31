import createClient from "openapi-fetch";
import type { paths, components } from "./party.d.ts";

export type { paths, components } from "./party.d.ts";

export type Party = components["schemas"]["Party"];
export type PartyView = components["schemas"]["PartyView"];
export type PartyCompany = components["schemas"]["PartyCompany"];
export type PartyIndividual = components["schemas"]["PartyIndividual"];
export type ExternalRef = components["schemas"]["ExternalRef"];
export type CreateCompanyRequest = components["schemas"]["CreateCompanyRequest"];
export type CreateIndividualRequest = components["schemas"]["CreateIndividualRequest"];
export type UpdatePartyRequest = components["schemas"]["UpdatePartyRequest"];
export type ListPartiesQuery = components["schemas"]["ListPartiesQuery"];
export type SearchQuery = components["schemas"]["SearchQuery"];
export type Contact = components["schemas"]["Contact"];
export type CreateContactRequest = components["schemas"]["CreateContactRequest"];
export type UpdateContactRequest = components["schemas"]["UpdateContactRequest"];
export type SetPrimaryRequest = components["schemas"]["SetPrimaryRequest"];
export type PrimaryContactEntry = components["schemas"]["PrimaryContactEntry"];
export type Address = components["schemas"]["Address"];
export type CreateAddressRequest = components["schemas"]["CreateAddressRequest"];
export type UpdateAddressRequest = components["schemas"]["UpdateAddressRequest"];
export type ApiError = components["schemas"]["ApiError"];
export type PaginatedResponseParty = components["schemas"]["PaginatedResponse_Party"];
export type PaginationMeta = components["schemas"]["PaginationMeta"];
export type DataResponseContact = components["schemas"]["DataResponse_Contact"];
export type DataResponseAddress = components["schemas"]["DataResponse_Address"];
export type DataResponsePrimaryContactEntry = components["schemas"]["DataResponse_PrimaryContactEntry"];

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
