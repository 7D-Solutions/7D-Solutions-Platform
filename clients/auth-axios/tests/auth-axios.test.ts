/**
 * Integration tests for @7d/auth-axios.
 *
 * Prerequisites: 7d-auth service on AUTH_BASE_URL (default http://localhost:8080).
 * Control-plane on CONTROL_PLANE_URL (default http://localhost:8091).
 * JWT_PRIVATE_KEY_PEM must be set to the platform RS256 private key.
 *
 * Test HTTP server: spun up in-process to provide controlled 401/200 responses
 * for the Bearer-intercept tests.  Refresh calls go to the real 7d-auth service.
 * This matches the pattern used by the Rust platform-sdk tests (axum test server).
 *
 * No mocks, no stubs.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { createServer, IncomingMessage, ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { randomUUID } from "node:crypto";
import { importPKCS8, SignJWT } from "jose";
import {
  createAuthAxios,
  attachAuthInterceptors,
  validateAxiosVersion,
} from "../src/index.js";
import { createAuthClient } from "@7d/auth-client";
import axios from "axios";

const AUTH_BASE_URL = process.env.AUTH_BASE_URL ?? "http://localhost:8080";
const CONTROL_PLANE_URL =
  process.env.CONTROL_PLANE_URL ?? "http://localhost:8091";

let TEST_TENANT_ID: string;
let testServerBaseUrl: string;

// ---------------------------------------------------------------------------
// Test HTTP server
//
// Returns 200 + { sessions: [] } when the Bearer token looks like a real JWT
// (3 dot-separated segments, first two segments >= 20 Base64URL chars —
// the length threshold cleanly separates real RS256 JWTs from short placeholder
// strings like "STALE_TOKEN").
//
// Returns 401 + { error: "token_expired" } otherwise.
//
// GET /api/always-fail -> always 500 (used by the non-401 passthrough test).
// ---------------------------------------------------------------------------

function isRealJwt(token: string): boolean {
  const parts = token.split(".");
  if (parts.length !== 3) return false;
  return parts[0].length >= 20 && parts[1].length >= 20;
}

const testServer = createServer(
  (req: IncomingMessage, res: ServerResponse) => {
    if (req.url?.startsWith("/api/always-fail")) {
      res.writeHead(500, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "internal_error" }));
      return;
    }

    const authHeader = (req.headers.authorization ?? "") as string;
    const tokenMatch = authHeader.match(/^Bearer (.+)$/);
    const token = tokenMatch?.[1] ?? "";

    if (isRealJwt(token)) {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ sessions: [] }));
    } else {
      res.writeHead(401, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "token_expired" }));
    }
  }
);

// ---------------------------------------------------------------------------
// Helpers (identical pattern to auth-client integration tests)
// ---------------------------------------------------------------------------

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
      idempotency_key: `auth-axios-test-${tenantId}`,
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
    email: `auth-axios-test-${userId.slice(0, 8)}@example.com`,
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
  await new Promise<void>((resolve) => testServer.listen(0, resolve));
  const port = (testServer.address() as AddressInfo).port;
  testServerBaseUrl = `http://localhost:${port}`;
  TEST_TENANT_ID = await provisionTestTenant();
}, 30_000);

afterAll(() => {
  testServer.close();
});

// ---------------------------------------------------------------------------
// validateAxiosVersion — pure-function unit tests (no network)
// ---------------------------------------------------------------------------

describe("validateAxiosVersion", () => {
  it("accepts version 1.0.0", () => {
    expect(() => validateAxiosVersion("1.0.0")).not.toThrow();
  });

  it("accepts version 2.0.0", () => {
    expect(() => validateAxiosVersion("2.0.0")).not.toThrow();
  });

  it("throws for version 0.27.2", () => {
    expect(() => validateAxiosVersion("0.27.2")).toThrow(
      "@7d/auth-axios requires axios ^1.0, found 0.27.2"
    );
  });

  it("throws for empty string", () => {
    expect(() => validateAxiosVersion("")).toThrow(
      "@7d/auth-axios requires axios ^1.0"
    );
  });
});

// ---------------------------------------------------------------------------
// Test 1: createAuthAxios + login → Bearer auto-attached → 200
// ---------------------------------------------------------------------------

describe("createAuthAxios — Bearer attach", () => {
  it("requests made after login carry a valid Bearer token and receive 200", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    const authClient = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
    });
    await authClient.login(user.email, user.password);

    const api = createAuthAxios({ baseURL: testServerBaseUrl, authClient });
    const { status } = await api.get("/api/protected");

    expect(status).toBe(200);
  });
});

// ---------------------------------------------------------------------------
// Test 2: expired token → interceptor refreshes via real 7d-auth → retries → 200
// ---------------------------------------------------------------------------

describe("createAuthAxios — 401 intercept triggers refresh and retry", () => {
  it("stale access token causes interceptor to refresh against real 7d-auth and succeed", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    const authClient = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
    });
    await authClient.login(user.email, user.password);

    // Overwrite with a non-JWT placeholder — test server will return 401.
    authClient.setAccessToken("STALE_TOKEN");

    const api = createAuthAxios({ baseURL: testServerBaseUrl, authClient });
    const { status } = await api.get("/api/protected");

    expect(status).toBe(200);
    // After refresh, the client holds a real JWT, not the stale placeholder.
    const newToken = authClient.getAccessToken();
    expect(newToken).toBeTruthy();
    expect(newToken).not.toBe("STALE_TOKEN");
    expect(isRealJwt(newToken!)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Test 3: 10 parallel 401s → exactly ONE /api/auth/refresh call
// ---------------------------------------------------------------------------

describe("createAuthAxios — refresh dedupe", () => {
  it("10 concurrent 401 responses produce exactly one /refresh call against real 7d-auth", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    let refreshCallCount = 0;
    const authClient = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
    });
    await authClient.login(user.email, user.password);

    const realRefresh = authClient.refresh.bind(authClient);
    authClient.refresh = async (): Promise<string> => {
      refreshCallCount++;
      return realRefresh();
    };

    authClient.setAccessToken("STALE_TOKEN");

    const api = createAuthAxios({ baseURL: testServerBaseUrl, authClient });

    const results = await Promise.all(
      Array.from({ length: 10 }, () => api.get("/api/protected"))
    );

    expect(refreshCallCount).toBe(1);
    for (const r of results) {
      expect(r.status).toBe(200);
    }
  });
});

// ---------------------------------------------------------------------------
// Test 4: revoked session → refresh returns 401 from real 7d-auth → onLogout fires once
// ---------------------------------------------------------------------------

describe("createAuthAxios — refresh failure calls onLogout once and surfaces 401", () => {
  it("when session is revoked, refresh fails, onLogout fires once, and the 401 propagates", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    let logoutCount = 0;
    const authClient = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
      onLogout: () => {
        logoutCount++;
      },
    });
    await authClient.login(user.email, user.password);

    // Revoke all sessions so that /refresh itself returns a non-OK response.
    const sessions = await authClient.listSessions();
    expect(sessions.length).toBeGreaterThan(0);
    await authClient.revokeSession(sessions[0].session_id);

    // Stale token forces a 401 from the test server, triggering the interceptor.
    authClient.setAccessToken("STALE_TOKEN");

    const api = createAuthAxios({ baseURL: testServerBaseUrl, authClient });

    let caughtStatus: number | undefined;
    try {
      await api.get("/api/protected");
    } catch (err: unknown) {
      if (axios.isAxiosError(err)) {
        caughtStatus = err.response?.status;
      }
    }

    expect(caughtStatus).toBe(401);
    expect(logoutCount).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// Test 5: non-401 response → interceptor does NOT call refresh
// ---------------------------------------------------------------------------

describe("createAuthAxios — non-401 passthrough", () => {
  it("a 500 response is propagated without calling refresh", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    let refreshCalled = false;
    const authClient = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
    });
    await authClient.login(user.email, user.password);

    const realRefresh = authClient.refresh.bind(authClient);
    authClient.refresh = async (): Promise<string> => {
      refreshCalled = true;
      return realRefresh();
    };

    const api = createAuthAxios({ baseURL: testServerBaseUrl, authClient });

    let caughtStatus: number | undefined;
    try {
      await api.get("/api/always-fail");
    } catch (err: unknown) {
      if (axios.isAxiosError(err)) {
        caughtStatus = err.response?.status;
      }
    }

    expect(caughtStatus).toBe(500);
    expect(refreshCalled).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Test 6: /api/auth/login 401 is terminal — interceptor does NOT refresh
// ---------------------------------------------------------------------------

describe("createAuthAxios — login endpoint 401 is terminal", () => {
  it("failed login with wrong credentials does not trigger refresh", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    let refreshCalled = false;
    const authClient = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
    });
    // Do NOT login — getAccessToken() returns null, so no Bearer header is sent.

    const realRefresh = authClient.refresh.bind(authClient);
    authClient.refresh = async (): Promise<string> => {
      refreshCalled = true;
      return realRefresh();
    };

    const api = createAuthAxios({ baseURL: AUTH_BASE_URL, authClient });

    let caughtStatus: number | undefined;
    try {
      await api.post("/api/auth/login", {
        tenant_id: user.tenantId,
        email: user.email,
        password: "WrongPassword!",
      });
    } catch (err: unknown) {
      if (axios.isAxiosError(err)) {
        caughtStatus = err.response?.status;
      }
    }

    // Login failure is a non-2xx response; refresh must never fire.
    expect(caughtStatus).toBeDefined();
    expect(refreshCalled).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Test 7: attachAuthInterceptors on a caller-owned instance works identically
// ---------------------------------------------------------------------------

describe("attachAuthInterceptors — caller-owned instance", () => {
  it("attaching interceptors to an existing instance provides the same behaviour as createAuthAxios", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    const authClient = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
    });
    await authClient.login(user.email, user.password);

    const instance = axios.create({
      baseURL: testServerBaseUrl,
      withCredentials: true,
    });
    attachAuthInterceptors(instance, authClient);

    // Stale token → test server returns 401 → interceptor refreshes → 200.
    authClient.setAccessToken("STALE_TOKEN");

    const { status } = await instance.get("/api/protected");

    expect(status).toBe(200);
    expect(isRealJwt(authClient.getAccessToken()!)).toBe(true);
  });
});
