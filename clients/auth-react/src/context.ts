import { createContext } from "react";
import type { AuthClient, LoginResponse } from "@7d/auth-client";

export interface AuthState {
  isAuthenticated: boolean;
  claims: Record<string, unknown>;
  accessToken: string | null;
}

export interface AuthContextValue extends AuthState {
  login: (username: string, password: string) => Promise<LoginResponse>;
  logout: () => Promise<void>;
  client: AuthClient;
  onLogout: (() => void) | undefined;
}

export const AuthContext = createContext<AuthContextValue | null>(null);
