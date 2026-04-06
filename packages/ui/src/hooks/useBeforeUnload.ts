import { useEffect } from "react";

/**
 * Shows a browser confirmation dialog when the user navigates away.
 * Only active when `enabled` is true — set to false when there is no unsaved work.
 */
export function useBeforeUnload(enabled: boolean): void {
  useEffect(() => {
    if (!enabled) return;

    const handler = (e: BeforeUnloadEvent) => {
      e.preventDefault();
      e.returnValue = "";
    };

    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, [enabled]);
}
