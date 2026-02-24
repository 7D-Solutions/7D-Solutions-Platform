// Toolbar customization store with localStorage persistence.

import { create } from 'zustand';
import { persist } from 'zustand/middleware';

export type ToolbarAction =
  | 'save'
  | 'undo'
  | 'redo'
  | 'zoomIn'
  | 'zoomOut'
  | 'fitWidth'
  | 'fitPage'
  | 'addStamp'
  | 'addText'
  | 'addHighlight'
  | 'delete'
  | 'print'
  | 'export';

export interface ToolbarButton {
  id: ToolbarAction;
  label: string;
  icon: string;
  shortcut?: string;
}

export const AVAILABLE_TOOLBAR_BUTTONS: ToolbarButton[] = [
  { id: 'save', label: 'Save', icon: 'save', shortcut: 'Ctrl+S' },
  { id: 'undo', label: 'Undo', icon: 'undo', shortcut: 'Ctrl+Z' },
  { id: 'redo', label: 'Redo', icon: 'redo', shortcut: 'Ctrl+Y' },
  { id: 'zoomIn', label: 'Zoom In', icon: 'zoom-in', shortcut: 'Ctrl++' },
  { id: 'zoomOut', label: 'Zoom Out', icon: 'zoom-out', shortcut: 'Ctrl+-' },
  { id: 'fitWidth', label: 'Fit Width', icon: 'fit-width' },
  { id: 'fitPage', label: 'Fit Page', icon: 'fit-page' },
  { id: 'addStamp', label: 'Add Stamp', icon: 'stamp' },
  { id: 'addText', label: 'Add Text', icon: 'text' },
  { id: 'addHighlight', label: 'Highlight', icon: 'highlight' },
  { id: 'delete', label: 'Delete', icon: 'trash', shortcut: 'Del' },
  { id: 'print', label: 'Print', icon: 'print', shortcut: 'Ctrl+P' },
  { id: 'export', label: 'Export', icon: 'export' },
];

const DEFAULT_TOOLBAR_BUTTONS: ToolbarAction[] = [
  'save',
  'undo',
  'redo',
  'print',
];

const TOOLBAR_VERSION = 1;

interface ToolbarStore {
  activeButtons: ToolbarAction[];
  isCustomizing: boolean;

  setActiveButtons: (buttons: ToolbarAction[]) => void;
  addButton: (buttonId: ToolbarAction) => void;
  removeButton: (buttonId: ToolbarAction) => void;
  reorderButtons: (fromIndex: number, toIndex: number) => void;
  resetToDefaults: () => void;
  setIsCustomizing: (isCustomizing: boolean) => void;
  getButtonConfig: (id: ToolbarAction) => ToolbarButton | undefined;
}

export const useToolbarStore = create<ToolbarStore>()(
  persist(
    (_set, _get) => ({
      activeButtons: DEFAULT_TOOLBAR_BUTTONS,
      isCustomizing: false,

      setActiveButtons: (buttons) => _set({ activeButtons: buttons }),

      addButton: (buttonId) =>
        _set((state) => {
          if (state.activeButtons.includes(buttonId)) return state;
          return { activeButtons: [...state.activeButtons, buttonId] };
        }),

      removeButton: (buttonId) =>
        _set((state) => ({
          activeButtons: state.activeButtons.filter((id) => id !== buttonId),
        })),

      reorderButtons: (fromIndex, toIndex) =>
        _set((state) => {
          const newButtons = [...state.activeButtons];
          const [removed] = newButtons.splice(fromIndex, 1);
          newButtons.splice(toIndex, 0, removed);
          return { activeButtons: newButtons };
        }),

      resetToDefaults: () => _set({ activeButtons: DEFAULT_TOOLBAR_BUTTONS }),

      setIsCustomizing: (isCustomizing) => _set({ isCustomizing }),

      getButtonConfig: (id) =>
        AVAILABLE_TOOLBAR_BUTTONS.find((btn) => btn.id === id),
    }),
    {
      name: 'pdf-toolbar-customization',
      version: TOOLBAR_VERSION,
    },
  ),
);
