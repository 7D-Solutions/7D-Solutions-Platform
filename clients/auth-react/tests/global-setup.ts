/**
 * Vitest globalSetup — runs in Node.js (not jsdom) before all tests.
 *
 * Mints an admin JWT and provisions one shared test tenant, then provides
 * the tenant ID to test files via inject().
 */

import { importPKCS8, SignJWT } from "jose";
import { randomUUID } from "node:crypto";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = fileURLToPath(new URL(".", import.meta.url));

// Load .env from the project root when running without an external env setup.
if (!process.env["JWT_PRIVATE_KEY_PEM"]) {
  try {
    process.loadEnvFile(resolve(__dirname, "../../../.env"));
  } catch {
    // .env not present in CI; env vars must be provided externally.
  }
}

const AUTH_BASE_URL = process.env.AUTH_BASE_URL ?? "http://localhost:8080";
const CONTROL_PLANE_URL = process.env.CONTROL_PLANE_URL ?? "http://localhost:8091";

async function mintAdminJwt(): Promise<string> {
  const pem = process.env.JWT_PRIVATE_KEY_PEM;
  if (!pem) throw new Error("JWT_PRIVATE_KEY_PEM env var is required");
  const privateKey = await importPKCS8(pem.replace(/\\n/g, "\n"), "RS256");
  const now = Math.floor(Date.now() / 1000);
  return new SignJWT({
    sub: randomUUID(),
    tenant_id: "00000000-0000-0000-0000-000000000000",
    iss: "auth-rs",
    aud: "7d-platform",
    iat: now,
    exp: now + 900,
    jti: randomUUID(),
    roles: ["admin"],
    perms: ["platform.tenants.create"],
    actor_type: "user",
    ver: "1",
  })
    .setProtectedHeader({ alg: "RS256" })
    .sign(privateKey);
}

async function provisionTestTenant(): Promise<string> {
  const tenantId = randomUUID();
  const token = await mintAdminJwt();
  const res = await fetch(`${CONTROL_PLANE_URL}/api/control/tenants`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({
      tenant_id: tenantId,
      idempotency_key: `auth-react-test-${tenantId}`,
      environment: "development",
      product_code: "starter",
      plan_code: "monthly",
      concurrent_user_limit: 10,
    }),
  });
  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Provision tenant failed ${res.status}: ${body}`);
  }
  return tenantId;
}

export async function setup({
  provide,
}: {
  provide: (key: string, value: unknown) => void;
}) {
  const tenantId = await provisionTestTenant();
  provide("TEST_TENANT_ID", tenantId);
  provide("AUTH_BASE_URL", AUTH_BASE_URL);
}
