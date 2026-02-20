// ============================================================
// Browser close/refresh warning hook
// Port from: docs/reference/fireproof/src/infrastructure/hooks/useBeforeUnload.ts
// Adapted: uses NEXT_PUBLIC_DISABLE_UNLOAD_WARNING instead of VITE_
// ============================================================
'use client';
import { useEffect } from 'react';

/**
 * Warns the user before closing or refreshing the tab when there are unsaved changes.
 * Disabled during E2E testing to prevent native browser popups from blocking Playwright.
 *
 * @param shouldWarn - Show warning when true
 * @param message - Optional custom message (most browsers show a generic message)
 */
export function useBeforeUnload(shouldWarn: boolean, message?: string): void {
  useEffect(() => {
    const isTestEnv =
      typeof process !== 'undefined' &&
      process.env.NEXT_PUBLIC_DISABLE_UNLOAD_WARNING === 'true';

    if (!shouldWarn || isTestEnv) return;

    const handleBeforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = message || '';
      return message || '';
    };

    window.addEventListener('beforeunload', handleBeforeUnload);
    return () => window.removeEventListener('beforeunload', handleBeforeUnload);
  }, [shouldWarn, message]);
}
