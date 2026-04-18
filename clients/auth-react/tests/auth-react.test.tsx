/**
 * Integration tests for @7d/auth-react.
 *
 * Prerequisites: 7d-auth service on AUTH_BASE_URL (default http://localhost:8080).
 * Control-plane on CONTROL_PLANE_URL (default http://localhost:8091).
 * JWT_PRIVATE_KEY_PEM must be set to the platform RS256 private key.
 *
 * Tenant provisioning (which requires JWT signing) runs in global-setup.ts
 * inside a plain Node.js process to avoid jsdom realm conflicts with crypto.
 *
 * All tests use real HTTP — no mocks, no stubs.
 */

import { describe, it, expect, beforeAll, inject } from "vitest";
import { render, act, waitFor, fireEvent } from "@testing-library/react";
import { StrictMode } from "react";
import { randomUUID } from "node:crypto";
import { createAuthClient } from "@7d/auth-client";
import {
  AuthProvider,
  useAuth,
  RequireAuth,
  useIdleTimeout,
  SessionsPanel,
} from "../src/index.js";

let TEST_TENANT_ID: string;
let AUTH_BASE_URL: string;

beforeAll(() => {
  TEST_TENANT_ID = inject("TEST_TENANT_ID") as string;
  AUTH_BASE_URL = (inject("AUTH_BASE_URL") as string) ?? "http://localhost:8080";
});

interface TestUser {
  email: string;
  password: string;
  tenantId: string;
  userId: string;
}

function makeTestUser(tenantId: string): TestUser {
  const userId = randomUUID();
  return {
    email: `auth-react-test-${userId.slice(0, 8)}@example.com`,
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

// ---------------------------------------------------------------------------
// AuthProvider — initial state
// ---------------------------------------------------------------------------

describe("AuthProvider - initial state", () => {
  it("isAuthenticated is false before login", () => {
    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: TEST_TENANT_ID,
    });

    function Child() {
      const { isAuthenticated } = useAuth();
      return <div data-testid="state">{String(isAuthenticated)}</div>;
    }

    const { getByTestId, unmount } = render(
      <AuthProvider client={client}>
        <Child />
      </AuthProvider>,
    );

    expect(getByTestId("state").textContent).toBe("false");
    unmount();
  });
});

// ---------------------------------------------------------------------------
// AuthProvider — login transitions
// ---------------------------------------------------------------------------

describe("AuthProvider - login", () => {
  it("login flips isAuthenticated true, populates claims, causes exactly one re-render", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
    });

    let renderCount = 0;
    let loginFn: ((email: string, pass: string) => Promise<unknown>) | null = null;

    function Child() {
      const { isAuthenticated, claims, login } = useAuth();
      renderCount++;
      loginFn = login;
      return (
        <div>
          <span data-testid="auth">{String(isAuthenticated)}</span>
          <span data-testid="sub">{String(claims["sub"] ?? "")}</span>
        </div>
      );
    }

    const { getByTestId, unmount } = render(
      <AuthProvider client={client}>
        <Child />
      </AuthProvider>,
    );

    const initialRenders = renderCount;
    expect(getByTestId("auth").textContent).toBe("false");

    await act(async () => {
      await loginFn!(user.email, user.password);
    });

    expect(getByTestId("auth").textContent).toBe("true");
    expect(getByTestId("sub").textContent).toBe(user.userId);
    // Exactly one state update per transition → one re-render.
    expect(renderCount - initialRenders).toBe(1);
    unmount();
  }, 20_000);
});

// ---------------------------------------------------------------------------
// RequireAuth
// ---------------------------------------------------------------------------

describe("RequireAuth - unauthenticated", () => {
  it("onLogout fires and children are hidden when not authenticated", async () => {
    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: TEST_TENANT_ID,
    });

    let logoutCalled = false;

    const { queryByText, unmount } = render(
      <AuthProvider
        client={client}
        onLogout={() => {
          logoutCalled = true;
        }}
      >
        <RequireAuth>
          <div>Protected content</div>
        </RequireAuth>
      </AuthProvider>,
    );

    expect(queryByText("Protected content")).toBeNull();

    // Flush effects so the useEffect inside RequireAuth fires.
    await act(async () => {});

    expect(logoutCalled).toBe(true);
    unmount();
  });
});

describe("RequireAuth - authenticated", () => {
  it("children render when client is logged in before mount", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
    });
    await client.login(user.email, user.password);

    const { getByText, unmount } = render(
      <AuthProvider client={client}>
        <RequireAuth>
          <div>Protected content</div>
        </RequireAuth>
      </AuthProvider>,
    );

    await waitFor(() => {
      expect(getByText("Protected content")).toBeTruthy();
    });
    unmount();
  }, 20_000);
});

// ---------------------------------------------------------------------------
// useIdleTimeout
// ---------------------------------------------------------------------------

describe("useIdleTimeout - validation", () => {
  it("throws a runtime error when minutes is 0", () => {
    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: TEST_TENANT_ID,
    });

    function Child() {
      useIdleTimeout({ minutes: 0, onTimeout: () => {} });
      return null;
    }

    expect(() => {
      render(
        <AuthProvider client={client}>
          <Child />
        </AuthProvider>,
      );
    }).toThrow("minutes must be > 0");
  });
});

describe("useIdleTimeout - fires exactly once after idle", () => {
  it("fires onTimeout once after minutes=0.05 (3s) of no activity", async () => {
    let timeoutCount = 0;

    function Child() {
      useIdleTimeout({
        minutes: 0.05,
        onTimeout: () => {
          timeoutCount++;
        },
      });
      return null;
    }

    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: TEST_TENANT_ID,
    });

    const { unmount } = render(
      <AuthProvider client={client}>
        <Child />
      </AuthProvider>,
    );

    // Wait 3.2s — just past the 3s idle window — with no activity.
    await new Promise((r) => setTimeout(r, 3200));

    expect(timeoutCount).toBe(1);
    unmount();
  }, 15_000);
});

describe("useIdleTimeout - activity prevents timeout", () => {
  it("does not fire when mousemove resets the timer every 500ms for 4.5s", async () => {
    let timeoutCount = 0;

    function Child() {
      useIdleTimeout({
        minutes: 0.05,
        onTimeout: () => {
          timeoutCount++;
        },
      });
      return null;
    }

    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: TEST_TENANT_ID,
    });

    const { unmount } = render(
      <AuthProvider client={client}>
        <Child />
      </AuthProvider>,
    );

    // Fire mousemove every 500ms for 4.5s (9 events).
    // The 3s idle window is reset on each event, so timeout never elapses.
    for (let i = 0; i < 9; i++) {
      await new Promise((r) => setTimeout(r, 500));
      window.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    }

    expect(timeoutCount).toBe(0);
    unmount();
  }, 20_000);
});

describe("useIdleTimeout - no re-renders on rapid activity", () => {
  it("100 mousemove events cause zero additional React re-renders", async () => {
    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: TEST_TENANT_ID,
    });

    let renderCount = 0;

    function Child() {
      const { isAuthenticated } = useAuth();
      renderCount++;
      useIdleTimeout({ minutes: 5, onTimeout: () => {} });
      return <div>{String(isAuthenticated)}</div>;
    }

    const { unmount } = render(
      <AuthProvider client={client}>
        <Child />
      </AuthProvider>,
    );

    // Allow effects to settle.
    await act(async () => {});

    const countBefore = renderCount;

    // Fire 100 mousemove events synchronously.
    for (let i = 0; i < 100; i++) {
      window.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    }

    // Synchronous dispatch + ref-based timestamp = no state updates.
    expect(renderCount).toBe(countBefore);
    unmount();
  });
});

describe("useIdleTimeout - StrictMode double-mount", () => {
  it("cleanup after first unmount leaves no duplicate timers; onTimeout fires exactly once", async () => {
    let timeoutCount = 0;
    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: TEST_TENANT_ID,
    });

    function Child() {
      useIdleTimeout({
        minutes: 0.05,
        onTimeout: () => {
          timeoutCount++;
        },
      });
      return null;
    }

    const { unmount } = render(
      <StrictMode>
        <AuthProvider client={client}>
          <Child />
        </AuthProvider>
      </StrictMode>,
    );

    // Wait one full idle window (3.2s). StrictMode double-mount should
    // leave exactly one active timer (the second-mount timer).
    await new Promise((r) => setTimeout(r, 3200));

    expect(timeoutCount).toBe(1);
    unmount();
  }, 15_000);
});

// ---------------------------------------------------------------------------
// SessionsPanel
// ---------------------------------------------------------------------------

describe("SessionsPanel - renders sessions and Revoke removes the row", () => {
  it("shows active sessions; clicking Revoke makes the row disappear on reload", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
    });
    await client.login(user.email, user.password);

    const sessions = await client.listSessions();
    expect(sessions.length).toBeGreaterThan(0);
    const firstSessionId = sessions[0].session_id;

    const { getByText, queryByTestId, unmount } = render(
      <AuthProvider client={client}>
        <SessionsPanel />
      </AuthProvider>,
    );

    // Wait for sessions to load.
    await waitFor(
      () => {
        expect(getByText("Revoke")).toBeTruthy();
      },
      { timeout: 10_000 },
    );

    // Click the first Revoke button.
    await act(async () => {
      fireEvent.click(getByText("Revoke"));
    });

    // After revoke + reload, that row's button should be gone.
    await waitFor(
      () => {
        expect(queryByTestId(`revoke-${firstSessionId}`)).toBeNull();
      },
      { timeout: 10_000 },
    );

    unmount();
  }, 30_000);
});

// ---------------------------------------------------------------------------
// Background refresh failure → provider clears state, onLogout fires
// ---------------------------------------------------------------------------

describe("Refresh failure propagates to provider", () => {
  it("provider becomes unauthenticated and onLogout fires when refresh returns 401", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    let logoutFired = false;
    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
    });
    await client.login(user.email, user.password);

    // Revoke the session so /refresh returns 401.
    const sessions = await client.listSessions();
    expect(sessions.length).toBeGreaterThan(0);
    await client.revokeSession(sessions[0].session_id);

    function Child() {
      const { isAuthenticated } = useAuth();
      return <div data-testid="auth">{String(isAuthenticated)}</div>;
    }

    const { getByTestId, unmount } = render(
      <AuthProvider
        client={client}
        onLogout={() => {
          logoutFired = true;
        }}
      >
        <Child />
      </AuthProvider>,
    );

    expect(getByTestId("auth").textContent).toBe("true");

    // The provider patches client.refresh. Calling it directly exercises
    // the patched version which syncs provider state on failure.
    await act(async () => {
      try {
        await client.refresh();
      } catch {
        // Expected: refresh fails with 401 (session revoked).
      }
    });

    await waitFor(
      () => {
        expect(getByTestId("auth").textContent).toBe("false");
      },
      { timeout: 10_000 },
    );

    expect(logoutFired).toBe(true);
    unmount();
  }, 30_000);
});

// ---------------------------------------------------------------------------
// Silent refresh — provider stays authenticated after background token swap
// ---------------------------------------------------------------------------

describe("Silent refresh through provider", () => {
  it("provider stays authenticated (no unmount, no flicker) after access token is refreshed in background", async () => {
    const user = makeTestUser(TEST_TENANT_ID);
    await registerUser(user);

    const client = createAuthClient({
      baseUrl: AUTH_BASE_URL,
      tenantId: user.tenantId,
    });
    await client.login(user.email, user.password);

    // Save a valid token so we can simulate the refresh path.
    const validToken = client.getAccessToken()!;

    let renderCount = 0;

    function Child() {
      const { isAuthenticated } = useAuth();
      renderCount++;
      return <div data-testid="auth">{String(isAuthenticated)}</div>;
    }

    const { getByTestId, unmount } = render(
      <AuthProvider client={client}>
        <Child />
      </AuthProvider>,
    );

    const rendersAfterMount = renderCount;
    expect(getByTestId("auth").textContent).toBe("true");

    // Simulate middleware restoring the access token (successful background refresh).
    await act(async () => {
      client.setAccessToken(validToken);
    });

    // Provider stays authenticated; no extra renders from the token swap.
    expect(getByTestId("auth").textContent).toBe("true");
    expect(renderCount).toBe(rendersAfterMount);
    unmount();
  }, 20_000);
});
