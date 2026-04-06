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

export const selectionStore = {
  subscribe(fn: Subscriber): () => void {
    subscribers.add(fn);
    return () => subscribers.delete(fn);
  },

  getSnapshot(): ReadonlyMap<string, Readonly<BucketState>> {
    return state;
  },

  select(key: string, item: unknown, id: string | number): void {
    const b = bucket(key);
    if (b.selectedIds.has(id)) return;
    b.selectedIds.add(id);
    b.selectedItems = [...b.selectedItems, item];
    notify();
  },

  deselect(key: string, id: string | number): void {
    const b = bucket(key);
    if (!b.selectedIds.has(id)) return;
    b.selectedIds.delete(id);
    b.selectedItems = b.selectedItems.filter((item) => {
      const itemId = (item as Record<string, unknown>)?.id;
      return itemId !== id;
    });
    notify();
  },

  toggle(key: string, item: unknown, id: string | number): void {
    const b = bucket(key);
    if (b.selectedIds.has(id)) {
      selectionStore.deselect(key, id);
    } else {
      selectionStore.select(key, item, id);
    }
  },

  selectAll<T>(key: string, items: T[], getId: (item: T) => string | number): void {
    const b = bucket(key);
    b.selectedIds = new Set(items.map(getId));
    b.selectedItems = [...items];
    notify();
  },

  deselectAll(key: string): void {
    const b = bucket(key);
    b.selectedIds = new Set();
    b.selectedItems = [];
    notify();
  },

  isSelected(
    snap: ReadonlyMap<string, Readonly<BucketState>>,
    key: string,
    id: string | number
  ): boolean {
    return snap.get(key)?.selectedIds.has(id) ?? false;
  },

  getCount(snap: ReadonlyMap<string, Readonly<BucketState>>, key: string): number {
    return snap.get(key)?.selectedIds.size ?? 0;
  },

  getSelected<T = unknown>(
    snap: ReadonlyMap<string, Readonly<BucketState>>,
    key: string
  ): T[] {
    return (snap.get(key)?.selectedItems ?? []) as T[];
  },

  isAllSelected(
    snap: ReadonlyMap<string, Readonly<BucketState>>,
    key: string,
    totalCount: number
  ): boolean {
    if (totalCount === 0) return false;
    return (snap.get(key)?.selectedIds.size ?? 0) >= totalCount;
  },
};
