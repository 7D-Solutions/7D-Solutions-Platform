/**
 * Integration proof: @7d/auth-client wired into @7d/ap-client.
 *
 * Prerequisites:
 *   - 7d-ap running on AP_BASE_URL (default http://localhost:8093)
 *   - 7d-auth running on AUTH_BASE_URL (default http://localhost:8080)
 *   - Control plane on CONTROL_PLANE_URL (default http://localhost:8091)
 *   - JWT_PRIVATE_KEY_PEM set to the platform RS256 private key
 *   - Auth postgres accessible via docker exec 7d-auth-postgres
 *
 * No mocks. No stubs. Real HTTP only.
 */

import { describe, it, expect, beforeAll } from "vitest";
import { randomUUID } from "node:crypto";
import { execSync } from "node:child_process";
import { importPKCS8, SignJWT } from "jose";
import { createAuthClient } from "@7d/auth-client";
import { createApClient } from "../src/index.js";

const AUTH_BASE_URL = process.env.AUTH_BASE_URL ?? "http://localhost:8080";
const AP_BASE_URL = process.env.AP_BASE_URL ?? "http://localhost:8093";
const CONTROL_PLANE_URL = process.env.CONTROL_PLANE_URL ?? "http://localhost:8091";

let TEST_TENANT_ID: string;

async function mintJwt(tenantId: string, perms: string[]): Promise<string> {
  const pem = process.env.JWT_PRIVATE_KEY_PEM;
  if (!pem) throw new Error("JWT_PRIVATE_KEY_PEM env var is required");
  const privateKey = await importPKCS8(pem.replace(/\\n/g, "\n"), "RS256");
  const now = Math.floor(Date.now() / 1000);
  return new SignJWT({
    sub: randomUUID(),
    tenant_id: tenantId,
    iss: "auth-rs",
    aud: "7d-platform",
    iat: now,
    exp: now + 900,
    jti: randomUUID(),
    roles: [],
    perms,
    actor_type: "user",
    ver: "1",
  })
    .setProtectedHeader({ alg: "RS256" })
    .sign(privateKey);
}

async function provisionTestTenant(): Promise<string> {
  const tenantId = randomUUID();
  const token = await mintJwt("00000000-0000-0000-0000-000000000000", [
    "platform.tenants.create",
  ]);
  const res = await fetch(`${CONTROL_PLANE_URL}/api/control/tenants`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({
      tenant_id: tenantId,
      idempotency_key: `ap-auth-proof-${tenantId}`,
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

interface TestUser {
  email: string;
  password: string;
  tenantId: string;
  userId: string;
}

function makeTestUser(tenantId: string): TestUser {
  const userId = randomUUID();
  return {
    email: `ap-auth-proof-${userId.slice(0, 8)}@example.com`,
    password: "TestPass123!",
    tenantId,
    userId,
  };
}

async function registerUser(u: TestUser): Promise<void> {
  const res = await fetch(`${AUTH_BASE_URL}/api/auth/register`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      tenant_id: u.tenantId,
      user_id: u.userId,
      email: u.email,
      password: u.password,
    }),
  });
  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Register failed ${res.status}: ${body}`);
  }
}

/**
 * Directly insert a role with ap.read + ap.mutate into the auth DB and bind
 * it to the user.  This is necessary because the auth service has no public
 * role-management API for test tenants.
 */
function grantApPermissions(tenantId: string, userId: string): void {
  const sql = `
DO $$
DECLARE
  v_role_id UUID;
  v_perm_id UUID;
BEGIN
  INSERT INTO roles (tenant_id, name, description, is_system)
  VALUES ('${tenantId}', 'ap-test-role', 'AP test role', false)
  ON CONFLICT (tenant_id, name) DO NOTHING;

  SELECT id INTO v_role_id
  FROM roles
  WHERE tenant_id = '${tenantId}' AND name = 'ap-test-role';

  FOR v_perm_id IN
    SELECT id FROM permissions WHERE key IN ('ap.read', 'ap.mutate')
  LOOP
    INSERT INTO role_permissions (role_id, permission_id)
    VALUES (v_role_id, v_perm_id)
    ON CONFLICT DO NOTHING;
  END LOOP;

  INSERT INTO user_role_bindings (tenant_id, user_id, role_id)
  VALUES ('${tenantId}', '${userId}', v_role_id)
  ON CONFLICT DO NOTHING;
END $$;
`;
  execSync("docker exec -i 7d-auth-postgres psql -U auth_user -d auth_db --no-psqlrc -q", {
    input: sql,
    stdio: ["pipe", "pipe", "pipe"],
  });
}

beforeAll(async () => {
  TEST_TENANT_ID = await provisionTestTenant();
}, 30_000);

// ---------------------------------------------------------------------------
// Test 1: authClient path — 5 successful AP calls with a minted token
// ---------------------------------------------------------------------------

describe("createApClient with authClient — happy path", () => {
  it("makes 5 successful AP calls using authClient for auth", async () => {
    const token = await mintJwt(TEST_TENANT_ID, ["ap.read", "ap.mutate"]);

    const authClient = createAuthClient({ baseUrl: AUTH_BASE_URL, tenantId: TEST_TENANT_ID });
    authClient.setAccessToken(token);

    const ap = createApClient({ baseUrl: AP_BASE_URL, authClient });

    for (let i = 0; i < 5; i++) {
      const { data, error, response } = await ap.GET("/api/ap/vendors", {
        params: { query: { limit: 10 } },
      });
      expect(response.status, `call ${i + 1} status`).toBe(200);
      expect(error).toBeUndefined();
      expect(data).toBeDefined();
    }
  });
});

// ---------------------------------------------------------------------------
// Test 2: middleware-driven transparent refresh on 401
// ---------------------------------------------------------------------------

describe("createApClient with authClient — transparent token refresh", () => {
  it("corrupted token triggers middleware refresh; next AP call succeeds", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);
    // Grant ap.read + ap.mutate directly in the auth DB so that refresh()
    // returns a token with the right permissions.
    grantApPermissions(user.tenantId, user.userId);

    const authClient = createAuthClient({ baseUrl: AUTH_BASE_URL, tenantId: user.tenantId });
    await authClient.login(user.email, user.password);

    const ap = createApClient({ baseUrl: AP_BASE_URL, authClient });

    // Verify baseline — valid token works.
    const baseline = await ap.GET("/api/ap/vendors", { params: { query: { limit: 1 } } });
    expect(baseline.response.status).toBe(200);

    // Corrupt the stored token so the next AP call returns 401.
    authClient.setAccessToken("invalid.jwt.token");

    // The middleware intercepts the 401, calls authClient.refresh() (which has
    // a stored refresh token from login), and retries — the result should be 200.
    const afterCorruption = await ap.GET("/api/ap/vendors", {
      params: { query: { limit: 1 } },
    });
    expect(afterCorruption.response.status).toBe(200);
    expect(afterCorruption.error).toBeUndefined();

    // The token in authClient was rotated by the refresh.
    const refreshedToken = authClient.getAccessToken();
    expect(refreshedToken).toBeTruthy();
    expect(refreshedToken).not.toBe("invalid.jwt.token");
  });
});

// ---------------------------------------------------------------------------
// Test 3: legacy static-token path still works
// ---------------------------------------------------------------------------

describe("createApClient with static token — legacy path", () => {
  it("static Bearer token is accepted and AP call returns 200", async () => {
    const token = await mintJwt(TEST_TENANT_ID, ["ap.read", "ap.mutate"]);
    const ap = createApClient({ baseUrl: AP_BASE_URL, token });

    const { data, error, response } = await ap.GET("/api/ap/vendors", {
      params: { query: { limit: 5 } },
    });
    expect(response.status).toBe(200);
    expect(error).toBeUndefined();
    expect(data).toBeDefined();
  });
});
