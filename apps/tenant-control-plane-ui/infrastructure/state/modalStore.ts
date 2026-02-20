// ============================================================
// Modal state management — tab-scoped, in-memory only
// Port from: docs/reference/fireproof/src/infrastructure/state/modalStore.ts
// Modals are scoped by tabId and routePath; not persisted.
// ============================================================
'use client';
import { create } from 'zustand';

export interface ModalState {
  id: string;
  tabId: string;
  type: string;
  props: Record<string, unknown>;
  routePath: string;
}

interface ModalStoreState {
  modals: ModalState[];

  openModal: (tabId: string, modalId: string, type: string, props: Record<string, unknown>, routePath: string) => void;
  closeModal: (modalId: string) => void;
  updateModalProps: (modalId: string, props: Record<string, unknown>) => void;
  closeAllModalsForTab: (tabId: string) => void;
  getModalsForTab: (tabId: string) => ModalState[];
  getModal: (modalId: string) => ModalState | undefined;
  isModalOpen: (modalId: string) => boolean;
}

export const useModalStore = create<ModalStoreState>()((set, get) => ({
  modals: [],

  openModal: (tabId, modalId, type, props, routePath) => {
    const { modals } = get();
    const existing = modals.find((m) => m.id === modalId);
    if (existing) {
      set({
        modals: modals.map((m) =>
          m.id === modalId ? { ...m, tabId, type, props: { ...m.props, ...props }, routePath } : m
        ),
      });
    } else {
      set({ modals: [...modals, { id: modalId, tabId, type, props, routePath }] });
    }
  },

  closeModal: (modalId) => {
    set((state) => ({ modals: state.modals.filter((m) => m.id !== modalId) }));
  },

  updateModalProps: (modalId, props) => {
    set((state) => ({
      modals: state.modals.map((m) =>
        m.id === modalId ? { ...m, props: { ...m.props, ...props } } : m
      ),
    }));
  },

  closeAllModalsForTab: (tabId) => {
    set((state) => ({ modals: state.modals.filter((m) => m.tabId !== tabId) }));
  },

  getModalsForTab: (tabId) => get().modals.filter((m) => m.tabId === tabId),
  getModal: (modalId) => get().modals.find((m) => m.id === modalId),
  isModalOpen: (modalId) => get().modals.some((m) => m.id === modalId),
}));

export const useModalsForTab = (tabId: string) =>
  useModalStore((state) => state.modals.filter((m) => m.tabId === tabId));

/**
 * Returns stable action references from the modal store.
 * Uses individual selectors so each returns a referentially-stable
 * function, avoiding the "getServerSnapshot should be cached" error
 * that occurs when a selector creates a new object every render.
 */
export function useModalActions() {
  const openModal = useModalStore((s) => s.openModal);
  const closeModal = useModalStore((s) => s.closeModal);
  const updateModalProps = useModalStore((s) => s.updateModalProps);
  const closeAllModalsForTab = useModalStore((s) => s.closeAllModalsForTab);
  const getModalsForTab = useModalStore((s) => s.getModalsForTab);
  const getModal = useModalStore((s) => s.getModal);
  const isModalOpen = useModalStore((s) => s.isModalOpen);

  return {
    openModal, closeModal, updateModalProps,
    closeAllModalsForTab, getModalsForTab, getModal, isModalOpen,
  };
}
