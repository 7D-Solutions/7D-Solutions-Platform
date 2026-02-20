// ============================================================
// Search state store factory — tab-scoped, localStorage-persisted
// Tracks search term and recent search history.
// ============================================================
'use client';
import { create } from 'zustand';
import type { UseBoundStore, StoreApi } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import { useActiveTabId } from './tabStore';

const MAX_RECENT_SEARCHES = 10;

interface SearchStoreState {
  searchTerm: string;
  recentSearches: string[];

  setSearchTerm: (term: string) => void;
  applySearch: (term: string) => void;
  clearSearch: () => void;
  clearRecentSearches: () => void;
}

const storeCache = new Map<string, UseBoundStore<StoreApi<unknown>>>();

/**
 * Tab-scoped, persistent search state factory.
 *
 * @example
 * const { searchTerm, setSearchTerm, recentSearches } = useSearchStore('tenant-list');
 */
export function useSearchStore(searchKey: string) {
  const activeTabId = useActiveTabId();
  const storageKey = `search-${searchKey}-${activeTabId}`;

  let store: UseBoundStore<StoreApi<SearchStoreState>>;
  if (storeCache.has(storageKey)) {
    store = storeCache.get(storageKey) as UseBoundStore<StoreApi<SearchStoreState>>;
  } else {
    store = create<SearchStoreState>()(
      persist(
        (set) => ({
          searchTerm: '',
          recentSearches: [],

          setSearchTerm: (term) => set({ searchTerm: term }),

          applySearch: (term) => {
            const trimmed = term.trim();
            set((state) => {
              if (!trimmed) return { searchTerm: '' };
              const filtered = state.recentSearches.filter((s) => s !== trimmed);
              return {
                searchTerm: trimmed,
                recentSearches: [trimmed, ...filtered].slice(0, MAX_RECENT_SEARCHES),
              };
            });
          },

          clearSearch: () => set({ searchTerm: '' }),

          clearRecentSearches: () => set({ recentSearches: [] }),
        }),
        {
          name: storageKey,
          storage: createJSONStorage(() => localStorage),
          partialize: (state) => ({
            searchTerm: state.searchTerm,
            recentSearches: state.recentSearches,
          }),
        }
      )
    );
    storeCache.set(storageKey, store);
  }

  return store();
}
