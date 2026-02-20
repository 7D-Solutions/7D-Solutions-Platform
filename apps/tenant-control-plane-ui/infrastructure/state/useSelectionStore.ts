// ============================================================
// Selection state store factory — tab-scoped, localStorage-persisted
// Multi-select / checkbox state for tables and lists.
// ============================================================
'use client';
import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import { useActiveTabId } from './tabStore';

interface SelectionStoreState {
  selectedItems: Set<string>;
  selectedCount: number;

  toggleItem: (id: string) => void;
  selectItem: (id: string) => void;
  deselectItem: (id: string) => void;
  selectAll: (ids: string[]) => void;
  clearSelection: () => void;
  isSelected: (id: string) => boolean;
}

const storeCache = new Map<string, ReturnType<typeof create>>();

/**
 * Tab-scoped, persistent selection state factory.
 *
 * @example
 * const { selectedItems, toggleItem, selectAll, selectedCount } = useSelectionStore('tenant-list');
 */
export function useSelectionStore(selectionKey: string) {
  const activeTabId = useActiveTabId();
  const storageKey = `selection-${selectionKey}-${activeTabId}`;

  let store: ReturnType<typeof create<SelectionStoreState>>;
  if (storeCache.has(storageKey)) {
    store = storeCache.get(storageKey) as ReturnType<typeof create<SelectionStoreState>>;
  } else {
    store = create<SelectionStoreState>()(
      persist(
        (set, get) => ({
          selectedItems: new Set<string>(),
          selectedCount: 0,

          toggleItem: (id) => {
            set((state) => {
              const next = new Set(state.selectedItems);
              if (next.has(id)) {
                next.delete(id);
              } else {
                next.add(id);
              }
              return { selectedItems: next, selectedCount: next.size };
            });
          },

          selectItem: (id) => {
            set((state) => {
              const next = new Set(state.selectedItems);
              next.add(id);
              return { selectedItems: next, selectedCount: next.size };
            });
          },

          deselectItem: (id) => {
            set((state) => {
              const next = new Set(state.selectedItems);
              next.delete(id);
              return { selectedItems: next, selectedCount: next.size };
            });
          },

          selectAll: (ids) => {
            const next = new Set(ids);
            set({ selectedItems: next, selectedCount: next.size });
          },

          clearSelection: () => set({ selectedItems: new Set(), selectedCount: 0 }),

          isSelected: (id) => get().selectedItems.has(id),
        }),
        {
          name: storageKey,
          storage: createJSONStorage(() => localStorage),
          partialize: (state) => ({ selectedItems: Array.from(state.selectedItems) }),
          onRehydrateStorage: () => (state) => {
            if (state) {
              const items = state.selectedItems as unknown as string[];
              state.selectedItems = new Set(Array.isArray(items) ? items : []);
              state.selectedCount = state.selectedItems.size;
            }
          },
        }
      )
    );
    storeCache.set(storageKey, store);
  }

  return store();
}
