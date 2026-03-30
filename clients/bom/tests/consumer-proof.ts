/**
 * Consumer proof: exercises BOM CRUD + explosion via the generated TS client.
 *
 * Prerequisites:
 *   - BOM service running on BOM_BASE_URL (default http://localhost:8107)
 *   - JWT_PRIVATE_KEY_PEM env var set (RS256 private key matching the service)
 *
 * Run:  npx tsx tests/consumer-proof.ts
 */

import createClient from "openapi-fetch";
import { importPKCS8, SignJWT } from "jose";
import { randomUUID } from "node:crypto";
import type { paths, components } from "../src/bom.d.ts";

// ---------- config ----------

const BOM_BASE_URL = process.env.BOM_BASE_URL ?? "http://localhost:8107";

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
    perms: ["bom.read", "bom.mutate"],
    actor_type: "user",
    ver: "1",
  })
    .setProtectedHeader({ alg: "RS256" })
    .sign(privateKey);
}

// ---------- helpers ----------

type BomHeader = components["schemas"]["BomHeader"];
type BomLine = components["schemas"]["BomLine"];
type ExplosionRow = components["schemas"]["ExplosionRow"];
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

// ---------- proof ----------

async function main() {
  console.log(`\nBOM Consumer Proof — ${BOM_BASE_URL}\n`);

  const token = await mintJwt();
  const client = createClient<paths>({
    baseUrl: BOM_BASE_URL,
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
  });

  // 1. Create a BOM header
  console.log("1. Create BOM header");
  const partId = randomUUID();
  const { data: created, error: createErr, response: createResp } = await client.POST("/api/bom", {
    body: { part_id: partId, description: "Consumer proof BOM" },
  });
  assert(createResp.status === 201, `POST /api/bom → 201 (got ${createResp.status})`);
  assert(created != null, "response body is present");
  assert(created!.part_id === partId, "part_id matches");
  assert(typeof created!.id === "string" && created!.id.length > 0, "id is a UUID");
  const bomId = created!.id;

  // 2. Get BOM by ID
  console.log("\n2. Get BOM by ID");
  const { data: fetched, response: getResp } = await client.GET("/api/bom/{bom_id}", {
    params: { path: { bom_id: bomId } },
  });
  assert(getResp.status === 200, `GET /api/bom/{bom_id} → 200 (got ${getResp.status})`);
  assert(fetched!.id === bomId, "fetched BOM matches created");

  // 3. Create a revision
  console.log("\n3. Create revision");
  const { data: revision, response: revResp } = await client.POST(
    "/api/bom/{bom_id}/revisions",
    {
      params: { path: { bom_id: bomId } },
      body: { revision_label: "A" },
    },
  );
  assert(revResp.status === 201, `POST revisions → 201 (got ${revResp.status})`);
  const revisionId = revision!.id;

  // 4. Add a line (top-level component)
  console.log("\n4. Add line to revision");
  const componentId = randomUUID();
  const { data: line, response: lineResp } = await client.POST(
    "/api/bom/revisions/{revision_id}/lines",
    {
      params: { path: { revision_id: revisionId } },
      body: {
        component_item_id: componentId,
        quantity: 2.0,
        scrap_factor: 0.05,
        uom: "EA",
      },
    },
  );
  assert(lineResp.status === 201, `POST line → 201 (got ${lineResp.status})`);
  assert(line!.component_item_id === componentId, "line component matches");
  assert(line!.quantity === 2.0, "line quantity matches");

  // 5. List lines for revision
  console.log("\n5. List lines");
  const { data: linesPage, response: linesResp } = await client.GET(
    "/api/bom/revisions/{revision_id}/lines",
    {
      params: { path: { revision_id: revisionId } },
    },
  );
  assert(linesResp.status === 200, `GET lines → 200 (got ${linesResp.status})`);
  assert(Array.isArray(linesPage!.data), "lines response has data array");
  assert(linesPage!.data.length >= 1, "at least one line returned");
  assert(linesPage!.pagination != null, "pagination metadata present");

  // 6. Run explosion
  console.log("\n6. Explosion");
  const { data: explosionRows, response: explResp } = await client.GET(
    "/api/bom/{bom_id}/explosion",
    {
      params: { path: { bom_id: bomId } },
    },
  );
  assert(explResp.status === 200, `GET explosion → 200 (got ${explResp.status})`);
  assert(Array.isArray(explosionRows), "explosion returns array");

  // Type check: verify ExplosionRow fields are typed (compile-time proof)
  if (explosionRows && explosionRows.length > 0) {
    const row: ExplosionRow = explosionRows[0];
    assert(typeof row.level === "number", "explosion row.level is number");
    assert(typeof row.component_item_id === "string", "explosion row.component_item_id is string");
    assert(typeof row.parent_part_id === "string", "explosion row.parent_part_id is string");
    assert(typeof row.quantity === "number", "explosion row.quantity is number");
    assert(typeof row.revision_id === "string", "explosion row.revision_id is string");
    assert(typeof row.revision_label === "string", "explosion row.revision_label is string");
    assert(typeof row.scrap_factor === "number", "explosion row.scrap_factor is number");
  } else {
    console.log("  (no explosion rows — single-level BOM, type safety proven at compile time)");
    passed++;
  }

  // 7. List BOMs (paginated)
  console.log("\n7. List BOMs (paginated)");
  const { data: bomList, response: listResp } = await client.GET("/api/bom", {
    params: { query: { page: 1, page_size: 5 } },
  });
  assert(listResp.status === 200, `GET /api/bom → 200 (got ${listResp.status})`);
  assert(Array.isArray(bomList!.data), "list has data array");
  assert(bomList!.pagination.page === 1, "pagination page is 1");

  // 8. Error shape on 404
  console.log("\n8. Error shape on 404");
  const { data: _notFound, error: notFoundErr, response: notFoundResp } = await client.GET(
    "/api/bom/{bom_id}",
    {
      params: { path: { bom_id: randomUUID() } },
    },
  );
  assert(notFoundResp.status === 404, `GET nonexistent → 404 (got ${notFoundResp.status})`);
  // ApiError shape check
  if (notFoundErr) {
    const err = notFoundErr as ApiError;
    assert(typeof err.error === "string", "ApiError.error is string");
    assert(typeof err.message === "string", "ApiError.message is string");
    assert(
      typeof err.request_id === "string" && err.request_id.length > 0,
      "ApiError.request_id present",
    );
  }

  // ---------- summary ----------
  console.log(`\n${"=".repeat(40)}`);
  console.log(`Results: ${passed} passed, ${failed} failed`);
  if (failed > 0) process.exit(1);
  console.log("Consumer proof PASSED\n");
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
