type Subscriber = () => void;

export interface ModalEntry {
  open: boolean;
  props?: Record<string, unknown>;
}

const state: Map<string, ModalEntry> = new Map();
const subscribers = new Set<Subscriber>();

function notify(): void {
  subscribers.forEach((fn) => fn());
}

/**
 * Lightweight module-level store for modal visibility.
 * Compatible with React.useSyncExternalStore.
 *
 * Usage:
 *   const snapshot = React.useSyncExternalStore(modalStore.subscribe, modalStore.getSnapshot);
 *   const isOpen = snapshot.get("confirm-delete")?.open ?? false;
 */
export const modalStore = {
  subscribe(fn: Subscriber): () => void {
    subscribers.add(fn);
    return () => subscribers.delete(fn);
  },

  getSnapshot(): ReadonlyMap<string, ModalEntry> {
    return state;
  },

  open(id: string, props?: Record<string, unknown>): void {
    state.set(id, { open: true, props });
    notify();
  },

  close(id: string): void {
    const current = state.get(id);
    if (current) {
      state.set(id, { ...current, open: false });
      notify();
    }
  },

  toggle(id: string): void {
    const current = state.get(id);
    if (current?.open) {
      modalStore.close(id);
    } else {
      modalStore.open(id, current?.props);
    }
  },

  isOpen(id: string): boolean {
    return state.get(id)?.open ?? false;
  },

  getProps(id: string): Record<string, unknown> | undefined {
    return state.get(id)?.props;
  },
};
