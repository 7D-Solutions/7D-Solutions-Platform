import { create } from "zustand";
import { type AccessClaims, decodeClaims } from "./claims";

export interface SessionState {
  accessToken: string | null;
  refreshToken: string | null;
  /** Decoded claims from the current access token. Null when not authenticated. */
  claims: AccessClaims | null;
  setTokens: (access: string, refresh: string) => void;
  clearSession: () => void;
}

/**
 * In-memory session store. Tokens live only for the current page session;
 * the app must re-authenticate after a hard reload.
 */
export const useSessionStore = create<SessionState>()((set) => ({
  accessToken: null,
  refreshToken: null,
  claims: null,

  setTokens(access, refresh) {
    set({
      accessToken: access,
      refreshToken: refresh,
      claims: decodeClaims(access),
    });
  },

  clearSession() {
    set({ accessToken: null, refreshToken: null, claims: null });
  },
}));
