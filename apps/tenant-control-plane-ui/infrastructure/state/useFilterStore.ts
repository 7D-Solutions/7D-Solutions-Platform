// ============================================================
// Filter state store factory — tab-scoped, localStorage-persisted
// Tracks active filters and provides hasActiveFilters detection.
// ============================================================
'use client';
import { create } from 'zustand';
import type { UseBoundStore, StoreApi } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import { useActiveTabId } from './tabStore';

interface FilterStoreState<T extends Record<string, unknown>> {
  filters: T;
  pendingFilters: T;
  hasActiveFilters: boolean;

  setFilter: <K extends keyof T>(key: K, value: T[K]) => void;
  setPendingFilter: <K extends keyof T>(key: K, value: T[K]) => void;
  applyFilters: () => void;
  clearFilters: () => void;
  resetPendingFilters: () => void;
}

const storeCache = new Map<string, UseBoundStore<StoreApi<unknown>>>();

function computeHasActiveFilters<T extends Record<string, unknown>>(
  filters: T,
  defaults: T
): boolean {
  return Object.keys(defaults).some((key) => {
    const val = filters[key];
    const def = defaults[key];
    return val !== def && val !== '' && val !== null && val !== undefined;
  });
}

/**
 * Tab-scoped, persistent filter state factory.
 *
 * @example
 * const { filters, setFilter, clearFilters, hasActiveFilters } = useFilterStore('tenant-list', {
 *   status: '', planId: ''
 * });
 */
export function useFilterStore<T extends Record<string, unknown>>(
  filterKey: string,
  defaultFilters: T
) {
  const activeTabId = useActiveTabId();
  const storageKey = `filter-${filterKey}-${activeTabId}`;

  let store: UseBoundStore<StoreApi<FilterStoreState<T>>>;
  if (storeCache.has(storageKey)) {
    store = storeCache.get(storageKey) as UseBoundStore<StoreApi<FilterStoreState<T>>>;
  } else {
    store = create<FilterStoreState<T>>()(
      persist(
        (set, get) => ({
          filters: defaultFilters,
          pendingFilters: defaultFilters,
          hasActiveFilters: false,

          setFilter: (key, value) => {
            set((state) => {
              const newFilters = { ...state.filters, [key]: value };
              return {
                filters: newFilters,
                pendingFilters: newFilters,
                hasActiveFilters: computeHasActiveFilters(newFilters, defaultFilters),
              };
            });
          },

          setPendingFilter: (key, value) => {
            set((state) => ({
              pendingFilters: { ...state.pendingFilters, [key]: value },
            }));
          },

          applyFilters: () => {
            set((state) => ({
              filters: state.pendingFilters,
              hasActiveFilters: computeHasActiveFilters(state.pendingFilters, defaultFilters),
            }));
          },

          clearFilters: () => {
            set({
              filters: defaultFilters,
              pendingFilters: defaultFilters,
              hasActiveFilters: false,
            });
          },

          resetPendingFilters: () => {
            set((state) => ({ pendingFilters: state.filters }));
          },
        }),
        {
          name: storageKey,
          storage: createJSONStorage(() => localStorage),
          partialize: (state) => ({ filters: state.filters }),
          onRehydrateStorage: () => (state) => {
            if (state) {
              state.pendingFilters = state.filters;
              state.hasActiveFilters = computeHasActiveFilters(state.filters, defaultFilters);
            }
          },
        }
      )
    );
    storeCache.set(storageKey, store);
  }

  return store();
}
