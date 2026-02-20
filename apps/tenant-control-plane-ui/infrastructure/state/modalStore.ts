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

export const useModalActions = () =>
  useModalStore((state) => ({
    openModal: state.openModal,
    closeModal: state.closeModal,
    updateModalProps: state.updateModalProps,
    closeAllModalsForTab: state.closeAllModalsForTab,
    getModalsForTab: state.getModalsForTab,
    getModal: state.getModal,
    isModalOpen: state.isModalOpen,
  }));
