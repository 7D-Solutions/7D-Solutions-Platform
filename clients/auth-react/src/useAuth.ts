import { useContext } from "react";
import { AuthContext } from "./context.js";
import type { AuthContextValue } from "./context.js";

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext);
  if (ctx === null) {
    throw new Error("useAuth must be used inside <AuthProvider>");
  }
  return ctx;
}
