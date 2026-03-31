/**
 * Consumer proof: exercises Party CRUD, contacts, and addresses via the generated client.
 *
 * Prerequisites:
 *   - Party service running on PARTY_BASE_URL (default http://localhost:8098)
 *   - JWT_PRIVATE_KEY_PEM env var set (RS256 private key matching the service)
 *
 * Run:  npx tsx tests/consumer-proof.ts
 */

import { config } from "dotenv";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import createClient from "openapi-fetch";
import { importPKCS8, SignJWT } from "jose";
import { randomUUID } from "node:crypto";
import type { paths, components } from "../src/party.d.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));
config({ path: resolve(__dirname, "../../../.env") });

const PARTY_BASE_URL = process.env.PARTY_BASE_URL ?? "http://localhost:8098";

async function mintJwt(): Promise<string> {
  const pem = process.env.JWT_PRIVATE_KEY_PEM;
  if (!pem) throw new Error("JWT_PRIVATE_KEY_PEM env var is required");

  const privateKey = await importPKCS8(pem, "RS256");
  const now = Math.floor(Date.now() / 1000);

  return new SignJWT({
    sub: randomUUID(),
    tenant_id: randomUUID(),
    iss: "auth-rs",
    aud: "7d-platform",
    iat: now,
    exp: now + 900,
    jti: randomUUID(),
    roles: ["admin"],
    perms: ["party.mutate"],
    actor_type: "user",
    ver: "1",
  })
    .setProtectedHeader({ alg: "RS256" })
    .sign(privateKey);
}

type ApiError = components["schemas"]["ApiError"];

let passed = 0;
let failed = 0;

function assert(condition: boolean, label: string): void {
  if (!condition) {
    console.error(`  FAIL: ${label}`);
    failed++;
  } else {
    console.log(`  PASS: ${label}`);
    passed++;
  }
}

async function main() {
  console.log(`\nParty Consumer Proof — ${PARTY_BASE_URL}\n`);

  const token = await mintJwt();
  const client = createClient<paths>({
    baseUrl: PARTY_BASE_URL,
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
  });

  const displayName = `Proof Party ${Date.now()}`;
  const legalName = `${displayName} LLC`;

  console.log("1. Create party company");
  const { data: created, response: createResp } = await client.POST("/api/party/companies", {
    body: {
      display_name: displayName,
      legal_name: legalName,
      email: "party-proof@7d.io",
      phone: "+1-512-555-0100",
      address_line1: "1 Proof Blvd",
      city: "Austin",
      state: "TX",
      country: "US",
    },
  });

  assert(createResp.status === 201, `POST /api/party/companies → 201 (got ${createResp.status})`);
  assert(created != null, "response body is present");
  if (!created) {
    console.error("Cannot continue without created party.");
    process.exit(1);
  }
  const partyId = created.id;
  assert(typeof partyId === "string" && partyId.length > 0, "party id is present");

  console.log("\n2. Get party by ID");
  const { data: fetched, response: getResp } = await client.GET("/api/party/parties/{id}", {
    params: { path: { id: partyId } },
  });
  assert(getResp.status === 200, `GET /api/party/parties/{id} → 200 (got ${getResp.status})`);
  assert(fetched?.id === partyId, "fetched party matches created");

  console.log("\n3. Update party details");
  const updatedName = `${displayName} (updated)`;
  const { data: updated, response: updateResp } = await client.PUT("/api/party/parties/{id}", {
    params: { path: { id: partyId } },
    body: {
      display_name: updatedName,
      website: "https://party.7d",
    },
  });
  assert(updateResp.status === 200, `PUT /api/party/parties/{id} → 200 (got ${updateResp.status})`);
  assert(updated?.display_name === updatedName, "display_name updated");

  console.log("\n4. Create contact");
  const { data: contact, response: contactResp } = await client.POST(
    "/api/party/parties/{party_id}/contacts",
    {
      params: { path: { party_id: partyId } },
      body: {
        first_name: "Proof",
        last_name: "Contact",
        email: "proof-contact@7d.io",
        phone: "+1-512-555-0101",
        role: "billing",
      },
    },
  );
  assert(contactResp.status === 201, `POST /contacts → 201 (got ${contactResp.status})`);
  assert(contact?.id, "contact id returned");

  console.log("\n5. List contacts yields DataResponse");
  const { data: contactsList, response: listContactsResp } = await client.GET(
    "/api/party/parties/{party_id}/contacts",
    {
      params: { path: { party_id: partyId } },
    },
  );
  assert(listContactsResp.status === 200, `GET /contacts → 200 (got ${listContactsResp.status})`);
  assert(Array.isArray(contactsList?.data), "data array exists");
  assert(
    contactsList?.data.some((entry) => entry.id === contact?.id),
    "created contact appears in list",
  );

  console.log("\n6. Set primary contact");
  await client.POST("/api/party/parties/{party_id}/contacts/{id}/set-primary", {
    params: { path: { party_id: partyId, id: contact!.id } },
    body: { role: "billing" },
  });

  console.log("\n7. Primary contacts returns DataResponse");
  const { data: primaryList, response: primaryResp } = await client.GET(
    "/api/party/parties/{party_id}/primary-contacts",
    {
      params: { path: { party_id: partyId } },
    },
  );
  assert(primaryResp.status === 200, `GET /primary-contacts → 200 (got ${primaryResp.status})`);
  assert(
    primaryList.data.some((entry) => entry.contact.id === contact?.id),
    "primary contact entry exists",
  );

  console.log("\n8. Create address");
  const { data: address, response: addressResp } = await client.POST(
    "/api/party/parties/{party_id}/addresses",
    {
      params: { path: { party_id: partyId } },
      body: {
        line1: "100 Proof Ave",
        city: "Austin",
        country: "US",
        label: "Proof HQ",
        is_primary: true,
      },
    },
  );
  assert(addressResp.status === 201, `POST /addresses → 201 (got ${addressResp.status})`);
  assert(address?.id, "address id returned");

  console.log("\n9. List addresses returns DataResponse");
  const { data: addressesList, response: addressesResp } = await client.GET(
    "/api/party/parties/{party_id}/addresses",
    {
      params: { path: { party_id: partyId } },
    },
  );
  assert(addressesResp.status === 200, `GET /addresses → 200 (got ${addressesResp.status})`);
  assert(
    addressesList?.data.some((entry) => entry.id === address?.id),
    "created address appears in list",
  );

  console.log("\n10. List parties (paginated)");
  const { data: partyList, response: listPartiesResp } = await client.GET("/api/party/parties", {
    params: { query: { page: 1, page_size: 5 } },
  });
  assert(listPartiesResp.status === 200, `GET /parties → 200 (got ${listPartiesResp.status})`);
  assert(Array.isArray(partyList?.data), "party list contains data array");

  console.log("\n11. Search by name");
  const { data: searchResult, response: searchResp } = await client.GET(
    "/api/party/parties/search",
    {
      params: { query: { name: updated?.display_name, limit: 5 } },
    },
  );
  assert(searchResp.status === 200, `GET /parties/search → 200 (got ${searchResp.status})`);
  assert(
    searchResult?.data.some((entry) => entry.id === partyId),
    "search returns created party",
  );

  console.log("\n12. 404 shape");
  const { error: notFoundErr, response: notFoundResp } = await client.GET(
    "/api/party/parties/{id}",
    {
      params: { path: { id: randomUUID() } },
    },
  );
  assert(notFoundResp.status === 404, `GET random → 404 (got ${notFoundResp.status})`);
  if (notFoundErr) {
    const err = notFoundErr as ApiError;
    assert(typeof err.error === "string", "ApiError.error is string");
    assert(typeof err.message === "string", "ApiError.message is string");
    assert(typeof err.request_id === "string", "ApiError.request_id present");
  }

  console.log(`\n${"=".repeat(40)}`);
  console.log(`Results: ${passed} passed, ${failed} failed`);
  if (failed > 0) process.exit(1);
  console.log("Consumer proof PASSED\n");
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
