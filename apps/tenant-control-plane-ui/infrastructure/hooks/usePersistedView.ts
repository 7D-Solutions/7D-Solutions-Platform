// ============================================================
// usePersistedView — per-table row/card view preference via BFF
// No localStorage. Preference is scoped by user + tableKey.
// ============================================================
'use client';
import { useState, useEffect, useCallback, useRef } from 'react';
import type { ViewMode } from '@/components/ui/ViewToggle';
import { userPreferencesService } from '@/infrastructure/services/userPreferencesService';

const DEFAULT_VIEW: ViewMode = 'row';

/**
 * Load and persist the row/card view preference for a given table.
 * Reads from and writes to the BFF preferences endpoint.
 *
 * @param tableKey - Unique identifier for the table (e.g. "tenants", "invoices")
 * @returns { viewMode, setViewMode, isLoading }
 */
export function usePersistedView(tableKey: string) {
  const [viewMode, setViewModeState] = useState<ViewMode>(DEFAULT_VIEW);
  const [isLoading, setIsLoading] = useState(true);
  const prefKey = `view-mode-${tableKey}`;
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    setIsLoading(true);

    userPreferencesService.getPreference<ViewMode>(prefKey, DEFAULT_VIEW).then((saved) => {
      if (mountedRef.current) {
        setViewModeState(saved ?? DEFAULT_VIEW);
        setIsLoading(false);
      }
    });

    return () => {
      mountedRef.current = false;
    };
  }, [prefKey]);

  const setViewMode = useCallback(
    (mode: ViewMode) => {
      setViewModeState(mode);
      userPreferencesService.savePreference(prefKey, mode);
    },
    [prefKey]
  );

  return { viewMode, setViewMode, isLoading } as const;
}
