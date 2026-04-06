type Subscriber = () => void;

interface BucketState {
  selectedIds: Set<string | number>;
  selectedItems: unknown[];
}

const state = new Map<string, BucketState>();
const subscribers = new Set<Subscriber>();

function notify(): void {
  subscribers.forEach((fn) => fn());
}

function bucket(key: string): BucketState {
  if (!state.has(key)) {
    state.set(key, { selectedIds: new Set(), selectedItems: [] });
  }
  return state.get(key)!;
}

/**
 * Module-level selection store for multi-select state.
 * Compatible with React.useSyncExternalStore.
 *
 * Each "selection key" is an independent bucket — one per list/table.
 *
 * @example
 *   const snap = React.useSyncExternalStore(selectionStore.subscribe, selectionStore.getSnapshot);
 *   const count = selectionStore.getCount(snap, "invoice-list");
 *   const isChecked = selectionStore.isSelected(snap, "invoice-list", row.id);
 *
 *   // Mutate
 *   selectionStore.toggle("invoice-list", row, row.id);
 *   selectionStore.selectAll("invoice-list", rows, (r) => r.id);
 *   selectionStore.deselectAll("invoice-list");
 */
export const selectionStore = {
  subscribe(fn: Subscriber): () => void {
    subscribers.add(fn);
    return () => subscribers.delete(fn);
  },

  getSnapshot(): ReadonlyMap<string, Readonly<BucketState>> {
    return state;
  },

  /** Select a single item. No-op if already selected. */
  select(key: string, item: unknown, id: string | number): void {
    const b = bucket(key);
    if (b.selectedIds.has(id)) return;
    b.selectedIds.add(id);
    b.selectedItems = [...b.selectedItems, item];
    notify();
  },

  /** Deselect a single item by id. */
  deselect(key: string, id: string | number): void {
    const b = bucket(key);
    if (!b.selectedIds.has(id)) return;
    b.selectedIds.delete(id);
    // Items may not have a standard id prop — filter by reference to selectedIds
    b.selectedItems = b.selectedItems.filter((item) => {
      const itemId = (item as Record<string, unknown>)?.id;
      return itemId !== id;
    });
    notify();
  },

  /** Toggle a single item. */
  toggle(key: string, item: unknown, id: string | number): void {
    const b = bucket(key);
    if (b.selectedIds.has(id)) {
      selectionStore.deselect(key, id);
    } else {
      selectionStore.select(key, item, id);
    }
  },

  /** Select all items, replacing any existing selection. */
  selectAll<T>(key: string, items: T[], getId: (item: T) => string | number): void {
    const b = bucket(key);
    b.selectedIds = new Set(items.map(getId));
    b.selectedItems = [...items];
    notify();
  },

  /** Clear the selection for a key. */
  deselectAll(key: string): void {
    const b = bucket(key);
    b.selectedIds = new Set();
    b.selectedItems = [];
    notify();
  },

  /** Whether an item is selected. Reads from the snapshot. */
  isSelected(
    snap: ReadonlyMap<string, Readonly<BucketState>>,
    key: string,
    id: string | number
  ): boolean {
    return snap.get(key)?.selectedIds.has(id) ?? false;
  },

  /** Number of selected items. Reads from the snapshot. */
  getCount(
    snap: ReadonlyMap<string, Readonly<BucketState>>,
    key: string
  ): number {
    return snap.get(key)?.selectedIds.size ?? 0;
  },

  /** Selected items array. Reads from the snapshot. */
  getSelected<T = unknown>(
    snap: ReadonlyMap<string, Readonly<BucketState>>,
    key: string
  ): T[] {
    return (snap.get(key)?.selectedItems ?? []) as T[];
  },

  /** Whether all items are selected (pass total visible count). */
  isAllSelected(
    snap: ReadonlyMap<string, Readonly<BucketState>>,
    key: string,
    totalCount: number
  ): boolean {
    if (totalCount === 0) return false;
    return (snap.get(key)?.selectedIds.size ?? 0) >= totalCount;
  },
};
