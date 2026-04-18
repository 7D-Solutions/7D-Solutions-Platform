import axios, { type AxiosInstance, type AxiosRequestConfig } from "axios";
import { createRefreshDeduplicator } from "@7d/auth-client";
import type { AuthClient } from "@7d/auth-client";

export type { AuthClient } from "@7d/auth-client";

// Augment axios config to carry the retry sentinel without casting everywhere.
declare module "axios" {
  interface InternalAxiosRequestConfig {
    __7d_auth_retried?: boolean;
  }
}

// Paths on which a 401 must never trigger a refresh attempt.
// - /api/auth/login: wrong-credentials 401 is fatal — caller must re-prompt.
// - /api/auth/refresh: a failing refresh is already handled inside authClient.refresh().
const AUTH_SKIP_PATHS = ["/api/auth/login", "/api/auth/refresh"];

export function validateAxiosVersion(version: string): void {
  const major = parseInt((version ?? "").split(".")[0] ?? "0", 10);
  if (isNaN(major) || major < 1) {
    throw new Error(
      `@7d/auth-axios requires axios ^1.0, found ${version ?? "unknown"}`
    );
  }
}

function isAuthSkipUrl(url: string): boolean {
  return AUTH_SKIP_PATHS.some((p) => url.includes(p));
}

export interface AuthAxiosOptions {
  baseURL: string;
  authClient: AuthClient;
  axiosConfig?: AxiosRequestConfig;
}

/**
 * Attach auth interceptors to an existing axios instance.
 *
 * Shared refresh deduplication is keyed to the authClient instance via
 * createRefreshDeduplicator — so all axios instances sharing the same
 * authClient will serialize through one /refresh call.
 */
export function attachAuthInterceptors(
  instance: AxiosInstance,
  authClient: AuthClient
): void {
  validateAxiosVersion(axios.VERSION);

  const dedup = createRefreshDeduplicator();

  // Attach Bearer token to every outgoing request.
  // Skip if no token yet (pre-login requests must not be authenticated).
  instance.interceptors.request.use((config) => {
    const token = authClient.getAccessToken();
    if (token) {
      config.headers.Authorization = `Bearer ${token}`;
    }
    return config;
  });

  // On 401: attempt exactly one silent refresh, then retry the original request.
  instance.interceptors.response.use(
    (response) => response,
    async (error: unknown) => {
      if (!axios.isAxiosError(error)) throw error;

      const config = error.config;
      const status = error.response?.status;

      // Only handle 401; everything else propagates unchanged.
      if (status !== 401 || !config) throw error;

      const url = config.url ?? "";
      // Auth-endpoint 401s are fatal — wrong credentials or revoked refresh.
      // Already-retried requests must not re-enter the refresh loop.
      if (config.__7d_auth_retried || isAuthSkipUrl(url)) throw error;

      let newToken: string;
      try {
        newToken = await dedup.run(() => authClient.refresh());
      } catch {
        // authClient.refresh() threw — session is dead, onLogout already fired
        // inside the auth client. Surface the original 401 to the caller.
        throw error;
      }

      config.__7d_auth_retried = true;
      // The request interceptor will pick up getAccessToken() on re-send, but
      // setting the header here ensures correctness even if the request
      // interceptor runs concurrently with another refresh cycle.
      config.headers.Authorization = `Bearer ${newToken}`;
      return instance(config);
    }
  );
}

/**
 * Create a preconfigured axios instance with auth interceptors installed.
 * withCredentials is set so the HttpOnly refresh cookie rides along on requests
 * to same-origin BFF endpoints.
 */
export function createAuthAxios(opts: AuthAxiosOptions): AxiosInstance {
  const instance = axios.create({
    baseURL: opts.baseURL,
    withCredentials: true,
    ...opts.axiosConfig,
  });
  attachAuthInterceptors(instance, opts.authClient);
  return instance;
}
