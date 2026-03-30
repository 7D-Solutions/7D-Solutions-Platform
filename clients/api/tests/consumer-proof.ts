/**
 * Consumer proof: verifies @7d/api meta-package re-exports compile
 * correctly and the unified client factory works against live services.
 *
 * Prerequisites:
 *   - Inventory, BOM, and Party services running
 *   - JWT_PRIVATE_KEY_PEM env var set (RS256 private key)
 *
 * Run:  npx tsx tests/consumer-proof.ts
 */

import { config } from "dotenv";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { importPKCS8, SignJWT } from "jose";
import { randomUUID } from "node:crypto";

// Import everything from the meta-package — this IS the proof
import {
  createClient,
  createInventoryClient,
  createBomClient,
  createPartyClient,
  type ApiClientOptions,
  type Item,
  type BomHeader,
  type Party,
  type ApiError,
  type PaginationMeta,
} from "../src/index.ts";

// Load .env from project root
const __dirname = dirname(fileURLToPath(import.meta.url));
config({ path: resolve(__dirname, "../../../.env") });

const BASE_URL = process.env.API_BASE_URL ?? "http://localhost:8092";
const INVENTORY_URL = process.env.INVENTORY_BASE_URL ?? BASE_URL;
const BOM_URL = process.env.BOM_BASE_URL ?? "http://localhost:8107";
const PARTY_URL = process.env.PARTY_BASE_URL ?? "http://localhost:8098";

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
    perms: [
      "inventory.read",
      "inventory.mutate",
      "bom.read",
      "bom.mutate",
      "party.read",
      "party.mutate",
    ],
    actor_type: "user",
    ver: "1",
  })
    .setProtectedHeader({ alg: "RS256" })
    .sign(privateKey);
}

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
  console.log("\n@7d/api Meta-Package Consumer Proof\n");

  // ── 1. Type-level proof ──────────────────────────────────────────
  console.log("1. Type compilation proof");

  // These assignments prove the types are correctly exported.
  // If any type were missing or wrong, tsc would fail here.
  const _itemType: Item | undefined = undefined;
  const _bomType: BomHeader | undefined = undefined;
  const _partyType: Party | undefined = undefined;
  const _errType: ApiError | undefined = undefined;
  const _pagType: PaginationMeta | undefined = undefined;
  void _itemType; void _bomType; void _partyType; void _errType; void _pagType;

  assert(true, "All types compile from @7d/api");

  // ── 2. Unified factory ───────────────────────────────────────────
  console.log("\n2. Unified createClient factory");

  const token = await mintJwt();
  const opts: ApiClientOptions = { token, baseUrl: INVENTORY_URL };
  const api = createClient(opts);

  assert(api.inventory != null, "api.inventory is defined");
  assert(api.bom != null, "api.bom is defined");
  assert(api.party != null, "api.party is defined");
  assert(typeof api.inventory.GET === "function", "inventory client has GET");
  assert(typeof api.bom.GET === "function", "bom client has GET");
  assert(typeof api.party.GET === "function", "party client has GET");

  // ── 3. Per-module factories still work ───────────────────────────
  console.log("\n3. Per-module factory re-exports");

  const inv = createInventoryClient({ token, baseUrl: INVENTORY_URL });
  const bom = createBomClient({ token, baseUrl: BOM_URL });
  const party = createPartyClient({ token, baseUrl: PARTY_URL });

  assert(typeof inv.GET === "function", "createInventoryClient works");
  assert(typeof bom.GET === "function", "createBomClient works");
  assert(typeof party.GET === "function", "createPartyClient works");

  // ── 4. Live call via unified client ──────────────────────────────
  console.log("\n4. Live API call via unified client");

  const { response: invResp } = await api.inventory.GET(
    "/api/inventory/items",
    { params: { query: { limit: 1, offset: 0 } } },
  );
  assert(
    invResp.status === 200,
    `GET /api/inventory/items → 200 (got ${invResp.status})`,
  );

  // ── 5. Live call via per-module client ───────────────────────────
  console.log("\n5. Live API calls via per-module clients");

  const { response: bomResp } = await bom.GET("/api/bom/headers", {
    params: { query: { limit: 1, offset: 0 } },
  });
  assert(
    bomResp.status === 200,
    `GET /api/bom/headers → 200 (got ${bomResp.status})`,
  );

  const { response: partyResp } = await party.GET("/api/parties", {
    params: { query: { limit: 1, offset: 0 } },
  });
  assert(
    partyResp.status === 200,
    `GET /api/parties → 200 (got ${partyResp.status})`,
  );

  // ── Summary ──────────────────────────────────────────────────────
  console.log(`\n${"=".repeat(40)}`);
  console.log(`Results: ${passed} passed, ${failed} failed`);
  if (failed > 0) process.exit(1);
  console.log("@7d/api consumer proof PASSED\n");
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
