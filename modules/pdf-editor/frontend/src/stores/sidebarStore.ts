// Sidebar mode store — controls which sidebar panel is active.

import { create } from 'zustand';

export type SidebarMode = 'TOOLS' | 'PAGES' | 'NOTES';

interface SidebarStore {
  mode: SidebarMode;
  activeModal: 'STAMP_PALETTE' | 'SIGNATURE_PALETTE' | 'BUBBLE_PALETTE' | null;

  setMode: (mode: SidebarMode) => void;
  setActiveModal: (modal: SidebarStore['activeModal']) => void;
  closeModal: () => void;
}

export const useSidebarStore = create<SidebarStore>((set) => ({
  mode: 'TOOLS',
  activeModal: null,

  setMode: (mode) => set({ mode }),
  setActiveModal: (activeModal) => set({ activeModal }),
  closeModal: () => set({ activeModal: null }),
}));
