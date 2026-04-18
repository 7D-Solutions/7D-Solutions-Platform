/**
 * Integration tests for @7d/auth-client.
 *
 * Prerequisites: 7d-auth service running on AUTH_BASE_URL (default http://localhost:8080).
 * Control-plane running on CONTROL_PLANE_URL (default http://localhost:8091).
 * JWT_PRIVATE_KEY_PEM must be set to the platform RS256 private key.
 *
 * All tests use real HTTP — no mocks, no stubs.
 */

import { describe, it, expect, beforeAll } from "vitest";
import { randomUUID } from "node:crypto";
import { importPKCS8, SignJWT } from "jose";
import { createAuthClient, createAuthMiddleware } from "../src/index.js";

const AUTH_BASE_URL = process.env.AUTH_BASE_URL ?? "http://localhost:8080";
const CONTROL_PLANE_URL = process.env.CONTROL_PLANE_URL ?? "http://localhost:8091";

// Shared test tenant — provisioned once per test run.
let TEST_TENANT_ID: string;

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
      idempotency_key: `auth-client-test-${tenantId}`,
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
    email: `auth-client-test-${userId.slice(0, 8)}@example.com`,
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

beforeAll(async () => {
  TEST_TENANT_ID = await provisionTestTenant();
}, 30_000);

// ---------------------------------------------------------------------------
// Test 1: login returns a non-empty access token; refresh cookie set
// ---------------------------------------------------------------------------

describe("createAuthClient — login", () => {
  it("login with valid credentials returns a non-empty access token", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    const client = createAuthClient({ baseUrl: AUTH_BASE_URL, tenantId: user.tenantId });
    const resp = await client.login(user.email, user.password);

    expect(resp.access_token).toBeTruthy();
    expect(resp.token_type).toBe("Bearer");
    expect(client.getAccessToken()).toBe(resp.access_token);
  });
});

// ---------------------------------------------------------------------------
// Test 2: middleware intercepts 401 → calls refresh → client gets a new token
// ---------------------------------------------------------------------------

describe("createAuthMiddleware — 401 intercept", () => {
  it("request with invalid token triggers refresh and client gets a new valid token", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    const authClient = createAuthClient({ baseUrl: AUTH_BASE_URL, tenantId: user.tenantId });
    await authClient.login(user.email, user.password);
    const originalToken = authClient.getAccessToken();

    // Corrupt the stored access token — next middleware-handled call will get 401.
    authClient.setAccessToken("invalid.jwt.token");

    const middleware = createAuthMiddleware(authClient);
    const fakeRequest = new Request(
      `${AUTH_BASE_URL}/api/auth/sessions?tenant_id=${user.tenantId}&user_id=${user.userId}`
    );
    const fake401 = new Response(JSON.stringify({ error: "unauthorized" }), {
      status: 401,
      headers: { "Content-Type": "application/json" },
    });

    await middleware.onResponse!({ request: fakeRequest, response: fake401, options: {} as never });

    const newToken = authClient.getAccessToken();
    expect(newToken).toBeTruthy();
    expect(newToken).not.toBe("invalid.jwt.token");
    expect(newToken).not.toBe(originalToken); // rotated by refresh
  });
});

// ---------------------------------------------------------------------------
// Test 3: 10 parallel 401s → exactly ONE POST /api/auth/refresh
// ---------------------------------------------------------------------------

describe("createAuthMiddleware — refresh dedupe", () => {
  it("10 concurrent 401 responses produce exactly one /refresh call", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    const authClient = createAuthClient({ baseUrl: AUTH_BASE_URL, tenantId: user.tenantId });
    await authClient.login(user.email, user.password);

    let refreshCallCount = 0;
    const realRefresh = authClient.refresh.bind(authClient);
    const countingClient = {
      ...authClient,
      refresh: async (): Promise<string> => {
        refreshCallCount++;
        return realRefresh();
      },
    };

    authClient.setAccessToken("invalid.jwt.token");

    const middleware = createAuthMiddleware(countingClient);
    const fakeRequest = new Request(
      `${AUTH_BASE_URL}/api/auth/sessions?tenant_id=${user.tenantId}&user_id=${user.userId}`
    );

    await Promise.all(
      Array.from({ length: 10 }, () =>
        middleware.onResponse!({
          request: fakeRequest,
          response: new Response(JSON.stringify({ error: "unauthorized" }), {
            status: 401,
            headers: { "Content-Type": "application/json" },
          }),
          options: {} as never,
        })
      )
    );

    expect(refreshCallCount).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// Test 4: revoke session → refresh returns 401 → onLogout fires exactly once
// ---------------------------------------------------------------------------

describe("createAuthMiddleware — refresh failure calls onLogout once", () => {
  it("when session is revoked and refresh returns 401, onLogout fires once", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    let logoutCount = 0;
    const authClient = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
      onLogout: () => { logoutCount++; },
    });
    await authClient.login(user.email, user.password);

    // Revoke the session so that /refresh itself returns 401.
    const sessions = await authClient.listSessions();
    expect(sessions.length).toBeGreaterThan(0);
    await authClient.revokeSession(sessions[0].session_id);

    // Corrupt access token so the intercepted call returns 401.
    authClient.setAccessToken("invalid.jwt.token");

    const middleware = createAuthMiddleware(authClient);
    const fakeRequest = new Request(
      `${AUTH_BASE_URL}/api/auth/sessions?tenant_id=${user.tenantId}&user_id=${user.userId}`
    );
    const fake401 = new Response(JSON.stringify({ error: "unauthorized" }), {
      status: 401,
      headers: { "Content-Type": "application/json" },
    });

    const result = await middleware.onResponse!({
      request: fakeRequest,
      response: fake401,
      options: {} as never,
    });

    expect(result?.status).toBe(401);
    expect(logoutCount).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// Test 5: non-401 errors do NOT trigger refresh
// ---------------------------------------------------------------------------

describe("createAuthMiddleware — non-401 passthrough", () => {
  it("a 500 response is returned as-is without calling refresh", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    let refreshCalled = false;
    const authClient = createAuthClient({ baseUrl: AUTH_BASE_URL, tenantId: user.tenantId });
    await authClient.login(user.email, user.password);

    const proxyClient = {
      ...authClient,
      refresh: async (): Promise<string> => {
        refreshCalled = true;
        return authClient.refresh();
      },
    };

    const middleware = createAuthMiddleware(proxyClient);
    const fake500 = new Response(JSON.stringify({ error: "internal" }), {
      status: 500,
      headers: { "Content-Type": "application/json" },
    });

    const result = await middleware.onResponse!({
      request: new Request(`${AUTH_BASE_URL}/some-endpoint`),
      response: fake500,
      options: {} as never,
    });

    expect(result?.status).toBe(500);
    expect(refreshCalled).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Test 6: logout → onLogout fires; subsequent refresh returns 401
// ---------------------------------------------------------------------------

describe("createAuthClient — logout", () => {
  it("after logout, onLogout fires and subsequent refresh fails with 401", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    let logoutFired = false;
    const authClient = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
      onLogout: () => { logoutFired = true; },
    });
    await authClient.login(user.email, user.password);
    await authClient.logout();

    expect(logoutFired).toBe(true);
    expect(authClient.getAccessToken()).toBeNull();

    await expect(authClient.refresh()).rejects.toThrow("Refresh failed: 401");
  });
});
