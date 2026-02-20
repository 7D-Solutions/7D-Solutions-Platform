// ============================================================
// View state store factory — tab-scoped, localStorage-persisted
// Active tab index, current wizard step, collapsed sections, view mode.
// ============================================================
'use client';
import { create } from 'zustand';
import type { UseBoundStore, StoreApi } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import { useActiveTabId } from './tabStore';

interface ViewStoreState<T extends Record<string, unknown>> {
  state: T;

  setState: (updates: Partial<T>) => void;
  resetState: () => void;
}

const storeCache = new Map<string, UseBoundStore<StoreApi<unknown>>>();

/**
 * Tab-scoped, persistent view state factory.
 * Use for: active tab index, current wizard step, collapsed sections.
 *
 * @example
 * const { state, setState } = useViewStore('tenant-detail', { activeTab: 0 });
 * const { activeTab } = state;
 */
export function useViewStore<T extends Record<string, unknown>>(
  viewKey: string,
  defaultState: T
) {
  const activeTabId = useActiveTabId();
  const storageKey = `view-${viewKey}-${activeTabId}`;

  let store: UseBoundStore<StoreApi<ViewStoreState<T>>>;
  if (storeCache.has(storageKey)) {
    store = storeCache.get(storageKey) as UseBoundStore<StoreApi<ViewStoreState<T>>>;
  } else {
    store = create<ViewStoreState<T>>()(
      persist(
        (set) => ({
          state: defaultState,

          setState: (updates) =>
            set((current) => ({ state: { ...current.state, ...updates } })),

          resetState: () => set({ state: defaultState }),
        }),
        {
          name: storageKey,
          storage: createJSONStorage(() => localStorage),
          partialize: (s) => ({ state: s.state }),
        }
      )
    );
    storeCache.set(storageKey, store);
  }

  return store();
}
