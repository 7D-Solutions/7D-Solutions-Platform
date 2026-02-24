// PDF tab store — manages multiple open PDFs with per-tab annotations.
// Annotations are stored in the browser only (no server persistence).
// Uses localStorage persistence for tab state across refreshes.

import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import type { Annotation } from '../api/types.ts';

export type ZoomLevel = 'fitWidth' | 'fitPage' | number;

export interface PdfTab {
  id: string;
  filename: string;
  pdfUrl: string;
  numPages: number;
  pageWidth?: number;
  pageHeight?: number;

  // Per-tab annotations (browser-local)
  annotations: Annotation[];
  bubbleOrder: string[];
  zoom: ZoomLevel;
  effectiveZoom?: number;
  rotation: 0 | 90 | 180 | 270;

  isDirty: boolean;

  // Undo/redo history
  history: { annotations: Annotation[]; bubbleOrder: string[] }[];
  historyIndex: number;
}

interface PdfTabStore {
  tabs: PdfTab[];
  activeTabId: string | null;
  _isUndoRedoOperation: boolean;

  createTab: (file: File) => string;
  closeTab: (id: string) => void;
  setActiveTab: (id: string) => void;
  updateActiveTab: (updates: Partial<PdfTab>) => void;
  getActiveTab: () => PdfTab | null;
  reorderTabs: (fromIndex: number, toIndex: number) => void;
  undo: () => void;
  redo: () => void;
  canUndo: () => boolean;
  canRedo: () => boolean;
}

let tabCounter = 0;
function nextTabId(): string {
  tabCounter += 1;
  return `tab-${Date.now()}-${tabCounter}`;
}

const TAB_STORAGE_VERSION = 1;

export const usePdfTabStore = create<PdfTabStore>()(
  persist(
    (set, get) => ({
      tabs: [],
      activeTabId: null,
      _isUndoRedoOperation: false,

      createTab: (file) => {
        const id = nextTabId();
        const pdfUrl = URL.createObjectURL(file);
        const newTab: PdfTab = {
          id,
          filename: file.name,
          pdfUrl,
          numPages: 0,
          annotations: [],
          bubbleOrder: [],
          zoom: 'fitWidth',
          rotation: 0,
          isDirty: false,
          history: [{ annotations: [], bubbleOrder: [] }],
          historyIndex: 0,
        };

        set((state) => ({
          tabs: [...state.tabs, newTab],
          activeTabId: id,
        }));

        return id;
      },

      closeTab: (id) => {
        const state = get();
        const newTabs = state.tabs.filter((t) => t.id !== id);
        let newActiveTabId = state.activeTabId;

        if (id === state.activeTabId) {
          newActiveTabId = newTabs.length > 0 ? newTabs[0].id : null;
        }

        set({ tabs: newTabs, activeTabId: newActiveTabId });
      },

      setActiveTab: (id) => set({ activeTabId: id }),

      updateActiveTab: (updates) => {
        const state = get();
        if (!state.activeTabId) return;

        set((s) => ({
          tabs: s.tabs.map((t) => {
            if (t.id !== state.activeTabId) return t;

            const updated = { ...t, ...updates };

            // Track annotation changes for undo/redo (skip during undo/redo)
            if (
              !state._isUndoRedoOperation &&
              ((updates.annotations && updates.annotations !== t.annotations) ||
                (updates.bubbleOrder && updates.bubbleOrder !== t.bubbleOrder))
            ) {
              const newHistory = t.history.slice(0, t.historyIndex + 1);
              newHistory.push({
                annotations: structuredClone(updated.annotations),
                bubbleOrder: structuredClone(updated.bubbleOrder || []),
              });
              updated.history = newHistory;
              updated.historyIndex = newHistory.length - 1;
            }

            return updated;
          }),
        }));
      },

      getActiveTab: () => {
        const state = get();
        return state.tabs.find((t) => t.id === state.activeTabId) ?? null;
      },

      reorderTabs: (fromIndex, toIndex) => {
        const newTabs = [...get().tabs];
        const [moved] = newTabs.splice(fromIndex, 1);
        newTabs.splice(toIndex, 0, moved);
        set({ tabs: newTabs });
      },

      undo: () => {
        const state = get();
        if (!state.activeTabId) return;
        const tab = state.tabs.find((t) => t.id === state.activeTabId);
        if (!tab || tab.historyIndex <= 0) return;

        const newIndex = tab.historyIndex - 1;
        const entry = tab.history[newIndex];

        set((s) => ({
          _isUndoRedoOperation: true,
          tabs: s.tabs.map((t) =>
            t.id === state.activeTabId
              ? {
                  ...t,
                  annotations: structuredClone(entry.annotations),
                  bubbleOrder: structuredClone(entry.bubbleOrder),
                  historyIndex: newIndex,
                }
              : t,
          ),
        }));

        Promise.resolve().then(() => set({ _isUndoRedoOperation: false }));
      },

      redo: () => {
        const state = get();
        if (!state.activeTabId) return;
        const tab = state.tabs.find((t) => t.id === state.activeTabId);
        if (!tab || tab.historyIndex >= tab.history.length - 1) return;

        const newIndex = tab.historyIndex + 1;
        const entry = tab.history[newIndex];

        set((s) => ({
          _isUndoRedoOperation: true,
          tabs: s.tabs.map((t) =>
            t.id === state.activeTabId
              ? {
                  ...t,
                  annotations: structuredClone(entry.annotations),
                  bubbleOrder: structuredClone(entry.bubbleOrder),
                  historyIndex: newIndex,
                }
              : t,
          ),
        }));

        Promise.resolve().then(() => set({ _isUndoRedoOperation: false }));
      },

      canUndo: () => {
        const state = get();
        if (!state.activeTabId) return false;
        const tab = state.tabs.find((t) => t.id === state.activeTabId);
        return tab ? tab.historyIndex > 0 : false;
      },

      canRedo: () => {
        const state = get();
        if (!state.activeTabId) return false;
        const tab = state.tabs.find((t) => t.id === state.activeTabId);
        return tab ? tab.historyIndex < tab.history.length - 1 : false;
      },
    }),
    {
      name: 'pdf-tabs-storage',
      version: TAB_STORAGE_VERSION,
      partialize: (state) => ({
        tabs: state.tabs.map((tab) => ({
          ...tab,
          // Don't persist history (too large) or blob URLs (not stable)
          history: [],
          historyIndex: 0,
          pdfUrl: '',
        })),
        activeTabId: state.activeTabId,
      }),
    },
  ),
);
