import type { Middleware } from "openapi-fetch";
import type { AuthClient, AuthClientOptions, LoginResponse, Session } from "./types.js";
import { createRefreshDeduplicator } from "./refresh-dedupe.js";

export type { AuthClient, AuthClientOptions, LoginResponse, Session } from "./types.js";
export { createRefreshDeduplicator, type RefreshDeduplicator } from "./refresh-dedupe.js";

function parseJwtClaims(token: string): Record<string, unknown> {
  try {
    const [, payload] = token.split(".");
    const padded = payload.replace(/-/g, "+").replace(/_/g, "/");
    const json = Buffer.from(padded, "base64").toString("utf8");
    return JSON.parse(json) as Record<string, unknown>;
  } catch {
    return {};
  }
}

function extractRefreshCookie(setCookieHeader: string | null): string | null {
  if (!setCookieHeader) return null;
  const m = setCookieHeader.match(/(?:^|,)\s*refresh=([^;,\s]+)/i);
  return m ? m[1] : null;
}

export function createAuthClient(opts: AuthClientOptions): AuthClient {
  const baseUrl = opts.baseUrl.replace(/\/$/, "");
  const tenantId = opts.tenantId;
  let accessToken: string | null = null;
  let storedRefreshCookie: string | null = null;
  let storedRefreshToken: string | null = null;
  let claims: Record<string, unknown> = {};

  function authFetch(path: string, init: RequestInit = {}): Promise<Response> {
    const headers: Record<string, string> = {
      ...(init.headers as Record<string, string> ?? {}),
    };
    // Node.js: inject the HttpOnly refresh cookie explicitly since native fetch
    // does not maintain a browser-style cookie jar. In a browser, credentials:'include'
    // handles this automatically and storedRefreshCookie stays null.
    if (storedRefreshCookie && !headers["Cookie"]) {
      headers["Cookie"] = `refresh=${storedRefreshCookie}`;
    }
    return fetch(`${baseUrl}${path}`, { ...init, credentials: "include", headers });
  }

  function storeTokens(data: LoginResponse, setCookieHeader: string | null): void {
    const cookie = extractRefreshCookie(setCookieHeader);
    if (cookie) storedRefreshCookie = cookie;
    if (data.refresh_token) storedRefreshToken = data.refresh_token;
    accessToken = data.access_token;
    claims = parseJwtClaims(data.access_token);
  }

  function clearTokens(): void {
    accessToken = null;
    storedRefreshCookie = null;
    storedRefreshToken = null;
    claims = {};
  }

  async function login(username: string, password: string): Promise<LoginResponse> {
    const res = await authFetch("/api/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ tenant_id: tenantId, email: username, password }),
    });
    if (!res.ok) throw new Error(`Login failed: ${res.status}`);
    const data = (await res.json()) as LoginResponse;
    storeTokens(data, res.headers.get("set-cookie"));
    return data;
  }

  async function refresh(): Promise<string> {
    if (!storedRefreshToken) {
      clearTokens();
      opts.onLogout?.();
      throw new Error("Refresh failed: 401");
    }
    const res = await authFetch("/api/auth/refresh", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ refresh_token: storedRefreshToken }),
    });
    if (!res.ok) {
      clearTokens();
      opts.onLogout?.();
      throw new Error(`Refresh failed: ${res.status}`);
    }
    const data = (await res.json()) as LoginResponse;
    storeTokens(data, res.headers.get("set-cookie"));
    return data.access_token;
  }

  async function logout(): Promise<void> {
    try {
      await authFetch("/api/auth/logout", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ tenant_id: tenantId, refresh_token: storedRefreshToken }),
      });
    } finally {
      clearTokens();
      opts.onLogout?.();
    }
  }

  function getAccessToken(): string | null {
    return accessToken;
  }

  function setAccessToken(token: string | null): void {
    accessToken = token;
    claims = token ? parseJwtClaims(token) : {};
  }

  async function listSessions(): Promise<Session[]> {
    const tenantId = claims["tenant_id"] as string | undefined;
    const userId = claims["sub"] as string | undefined;
    if (!tenantId || !userId) throw new Error("Not logged in");
    const params = new URLSearchParams({ tenant_id: tenantId, user_id: userId });
    const res = await fetch(`${baseUrl}/api/auth/sessions?${params}`, {
      credentials: "include",
      headers: accessToken ? { Authorization: `Bearer ${accessToken}` } : {},
    });
    if (!res.ok) throw new Error(`List sessions failed: ${res.status}`);
    const data = (await res.json()) as { sessions: Session[] };
    return data.sessions;
  }

  async function revokeSession(sessionId: string): Promise<void> {
    const tenantId = claims["tenant_id"] as string | undefined;
    const userId = claims["sub"] as string | undefined;
    if (!tenantId || !userId) throw new Error("Not logged in");
    const res = await fetch(`${baseUrl}/api/auth/sessions/${sessionId}/revoke`, {
      method: "POST",
      credentials: "include",
      headers: {
        "Content-Type": "application/json",
        ...(accessToken ? { Authorization: `Bearer ${accessToken}` } : {}),
      },
      body: JSON.stringify({ tenant_id: tenantId, user_id: userId }),
    });
    if (!res.ok) throw new Error(`Revoke session failed: ${res.status}`);
  }

  return { login, logout, refresh, getAccessToken, setAccessToken, listSessions, revokeSession };
}

export function createAuthMiddleware(authClient: AuthClient): Middleware {
  // One deduplicator per middleware instance. /refresh rotates the cookie on every call —
  // two concurrent /refresh calls from the same cookie cause a replay-detection revoke.
  const dedup = createRefreshDeduplicator();

  return {
    onRequest({ request }) {
      const token = authClient.getAccessToken();
      if (token) request.headers.set("Authorization", `Bearer ${token}`);
      return request;
    },

    async onResponse({ request, response }) {
      if (response.status !== 401) return response;

      let newToken: string;
      try {
        newToken = await dedup.run(() => authClient.refresh());
      } catch {
        // refresh() itself returned 401 — session revoked; onLogout already fired
        return response;
      }

      // Retry the original request exactly once with the fresh token.
      const retried = new Request(request.url, {
        method: request.method,
        headers: new Headers(request.headers),
        body: request.body,
        mode: request.mode,
        credentials: request.credentials,
        cache: request.cache,
        redirect: request.redirect,
        referrer: request.referrer,
        integrity: request.integrity,
      });
      retried.headers.set("Authorization", `Bearer ${newToken}`);
      return fetch(retried);
    },
  };
}
