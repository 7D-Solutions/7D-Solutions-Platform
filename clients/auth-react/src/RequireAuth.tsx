import { useEffect, type ReactNode } from "react";
import { useAuth } from "./useAuth.js";

interface RequireAuthProps {
  children: ReactNode;
}

export function RequireAuth({ children }: RequireAuthProps): ReactNode {
  const { isAuthenticated, onLogout } = useAuth();

  useEffect(() => {
    if (!isAuthenticated) {
      onLogout?.();
    }
  }, [isAuthenticated, onLogout]);

  if (!isAuthenticated) return null;
  return <>{children}</>;
}
