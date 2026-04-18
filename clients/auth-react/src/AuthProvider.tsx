import { useState, useCallback, useEffect, useRef, useMemo } from "react";
import type { ReactNode } from "react";
import type { AuthClient, LoginResponse } from "@7d/auth-client";
import { AuthContext } from "./context.js";
import type { AuthContextValue, AuthState } from "./context.js";

function parseJwtClaims(token: string | null): Record<string, unknown> {
  if (!token) return {};
  try {
    const [, payload] = token.split(".");
    const padded = payload.replace(/-/g, "+").replace(/_/g, "/");
    return JSON.parse(atob(padded)) as Record<string, unknown>;
  } catch {
    return {};
  }
}

function snapshotState(client: AuthClient): AuthState {
  const accessToken = client.getAccessToken();
  return {
    isAuthenticated: accessToken !== null,
    claims: parseJwtClaims(accessToken),
    accessToken,
  };
}

export interface AuthProviderProps {
  client: AuthClient;
  onLogout?: () => void;
  children: ReactNode;
}

export function AuthProvider({ client, onLogout, children }: AuthProviderProps) {
  const [authState, setAuthState] = useState<AuthState>(() => snapshotState(client));
  const onLogoutRef = useRef(onLogout);

  // Keep ref current on every render without re-running the patch effect.
  useEffect(() => {
    onLogoutRef.current = onLogout;
  });

  // Patch client.refresh so background failures (triggered by middleware) update
  // provider state. The middleware calls client.refresh() directly; this is the
  // only state transition that doesn't go through the context wrapper functions.
  useEffect(() => {
    const origRefresh = client.refresh.bind(client);
    client.refresh = async (): Promise<string> => {
      try {
        return await origRefresh();
      } catch (err) {
        // auth-client already called clearTokens(); sync provider state.
        setAuthState(snapshotState(client));
        onLogoutRef.current?.();
        throw err;
      }
    };
    return () => {
      client.refresh = origRefresh;
    };
  }, [client]);

  const login = useCallback(
    async (username: string, password: string): Promise<LoginResponse> => {
      const result = await client.login(username, password);
      setAuthState(snapshotState(client));
      return result;
    },
    [client],
  );

  const logout = useCallback(async (): Promise<void> => {
    await client.logout();
    setAuthState(snapshotState(client));
    onLogoutRef.current?.();
  }, [client]);

  const value = useMemo<AuthContextValue>(
    () => ({ ...authState, login, logout, client, onLogout }),
    [authState, login, logout, client, onLogout],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}
