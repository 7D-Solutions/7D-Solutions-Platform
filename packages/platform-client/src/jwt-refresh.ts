import { ApiError } from "./error";
import { useSessionStore } from "./session-store";

export interface TokenResponse {
  access_token: string;
  refresh_token: string;
}

/**
 * Calls POST /api/auth/refresh against identity-auth, rotates tokens in the
 * session store, and clears the session on failure.
 */
export async function refreshSession(identityAuthBaseUrl: string): Promise<void> {
  const { refreshToken, setTokens, clearSession } = useSessionStore.getState();

  if (!refreshToken) {
    clearSession();
    throw new ApiError(401, "no_refresh_token", "No refresh token available");
  }

  const res = await fetch(`${identityAuthBaseUrl}/api/auth/refresh`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ refresh_token: refreshToken }),
  });

  if (!res.ok) {
    clearSession();
    const body = await res.json().catch(() => ({})) as unknown;
    throw ApiError.fromResponse(res.status, body);
  }

  const data = (await res.json()) as TokenResponse;
  setTokens(data.access_token, data.refresh_token);
}
