// ============================================================
// Tab context hook — gives screens access to their tab's state.
// Use this to mark a tab dirty on first edit and read tab id.
// ============================================================
'use client';
import { useCallback } from 'react';
import { useActiveTabId, useTabStore } from '../state/tabStore';

export interface TabContext {
  tabId: string;
  isDirty: boolean;
  setDirty: (dirty: boolean) => void;
  promote: () => void;
}

/**
 * Provides the current tab context — id, dirty state, and actions.
 *
 * @example
 * const { tabId, isDirty, setDirty, promote } = useTabContext();
 * // On first form edit:
 * setDirty(true);
 */
export function useTabContext(): TabContext {
  const tabId = useActiveTabId();
  const tab = useTabStore((s) => s.tabs.find((t) => t.id === tabId));
  const updateTab = useTabStore((s) => s.updateTab);

  const setDirty = useCallback(
    (dirty: boolean) => updateTab(tabId, { isDirty: dirty }),
    [tabId, updateTab]
  );

  const promote = useCallback(
    () => updateTab(tabId, { isPreview: false }),
    [tabId, updateTab]
  );

  return {
    tabId,
    isDirty: tab?.isDirty ?? false,
    setDirty,
    promote,
  };
}
