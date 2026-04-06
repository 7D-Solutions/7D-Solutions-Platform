import { ApiError } from "./error";
import { useSessionStore } from "./session-store";
import { refreshSession } from "./jwt-refresh";

export interface AuthFetcherOptions {
  /** Base URL of the identity-auth service, used when a 401 triggers a token refresh. */
  identityAuthBaseUrl: string;
}

export type AuthFetch = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

/**
 * Returns a `fetch`-compatible function that:
 * 1. Attaches `Authorization: Bearer <accessToken>` from the session store.
 * 2. On 401, refreshes the token once and retries the original request.
 * 3. Throws `ApiError(401)` if the refresh also fails.
 */
export function createAuthFetcher(options: AuthFetcherOptions): AuthFetch {
  return async function authFetch(input, init) {
    const { accessToken } = useSessionStore.getState();

    const headers = new Headers(init?.headers);
    if (accessToken) {
      headers.set("Authorization", `Bearer ${accessToken}`);
    }

    const res = await fetch(input, { ...init, headers });

    if (res.status !== 401) {
      return res;
    }

    // One refresh attempt
    try {
      await refreshSession(options.identityAuthBaseUrl);
    } catch {
      throw new ApiError(401, "session_expired", "Session expired — please log in again");
    }

    const { accessToken: newToken } = useSessionStore.getState();
    const retryHeaders = new Headers(init?.headers);
    if (newToken) {
      retryHeaders.set("Authorization", `Bearer ${newToken}`);
    }
    return fetch(input, { ...init, headers: retryHeaders });
  };
}
