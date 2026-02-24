// Smoke tests for the PDF tab store.
// Tests browser-local tab management and undo/redo.

import { describe, it, expect, beforeEach } from 'vitest';
import { usePdfTabStore } from '../stores/pdfTabStore.ts';

function resetStore() {
  usePdfTabStore.setState({
    tabs: [],
    activeTabId: null,
    _isUndoRedoOperation: false,
  });
}

describe('pdfTabStore', () => {
  beforeEach(resetStore);

  it('starts with no tabs', () => {
    const state = usePdfTabStore.getState();
    expect(state.tabs).toEqual([]);
    expect(state.activeTabId).toBeNull();
  });

  it('getActiveTab returns null when no tabs', () => {
    expect(usePdfTabStore.getState().getActiveTab()).toBeNull();
  });

  it('closes a non-existent tab without error', () => {
    usePdfTabStore.getState().closeTab('nonexistent');
    expect(usePdfTabStore.getState().tabs).toEqual([]);
  });

  it('switches active tab', () => {
    // Manually create tabs for testing (createTab needs File which needs DOM)
    usePdfTabStore.setState({
      tabs: [
        { id: 'tab-1', filename: 'a.pdf', pdfUrl: '', numPages: 1, annotations: [], bubbleOrder: [], zoom: 'fitWidth', rotation: 0, isDirty: false, history: [{ annotations: [], bubbleOrder: [] }], historyIndex: 0 },
        { id: 'tab-2', filename: 'b.pdf', pdfUrl: '', numPages: 2, annotations: [], bubbleOrder: [], zoom: 'fitWidth', rotation: 0, isDirty: false, history: [{ annotations: [], bubbleOrder: [] }], historyIndex: 0 },
      ],
      activeTabId: 'tab-1',
    });

    usePdfTabStore.getState().setActiveTab('tab-2');
    expect(usePdfTabStore.getState().activeTabId).toBe('tab-2');
  });

  it('closes active tab and falls back to first remaining', () => {
    usePdfTabStore.setState({
      tabs: [
        { id: 'tab-1', filename: 'a.pdf', pdfUrl: '', numPages: 1, annotations: [], bubbleOrder: [], zoom: 'fitWidth', rotation: 0, isDirty: false, history: [{ annotations: [], bubbleOrder: [] }], historyIndex: 0 },
        { id: 'tab-2', filename: 'b.pdf', pdfUrl: '', numPages: 2, annotations: [], bubbleOrder: [], zoom: 'fitWidth', rotation: 0, isDirty: false, history: [{ annotations: [], bubbleOrder: [] }], historyIndex: 0 },
      ],
      activeTabId: 'tab-1',
    });

    usePdfTabStore.getState().closeTab('tab-1');
    expect(usePdfTabStore.getState().tabs).toHaveLength(1);
    expect(usePdfTabStore.getState().activeTabId).toBe('tab-2');
  });

  it('updates active tab annotations', () => {
    const ann = { id: 'ann-1', x: 10, y: 20, pageNumber: 1, type: 'TEXT' as const };
    usePdfTabStore.setState({
      tabs: [
        { id: 'tab-1', filename: 'a.pdf', pdfUrl: '', numPages: 1, annotations: [], bubbleOrder: [], zoom: 'fitWidth', rotation: 0, isDirty: false, history: [{ annotations: [], bubbleOrder: [] }], historyIndex: 0 },
      ],
      activeTabId: 'tab-1',
    });

    usePdfTabStore.getState().updateActiveTab({ annotations: [ann] });
    const tab = usePdfTabStore.getState().getActiveTab();
    expect(tab?.annotations).toHaveLength(1);
    expect(tab?.annotations[0].id).toBe('ann-1');
    // History should have a new entry
    expect(tab?.historyIndex).toBe(1);
  });

  it('undo reverts annotations', async () => {
    const ann = { id: 'ann-1', x: 10, y: 20, pageNumber: 1, type: 'TEXT' as const };
    usePdfTabStore.setState({
      tabs: [
        { id: 'tab-1', filename: 'a.pdf', pdfUrl: '', numPages: 1, annotations: [], bubbleOrder: [], zoom: 'fitWidth', rotation: 0, isDirty: false, history: [{ annotations: [], bubbleOrder: [] }], historyIndex: 0 },
      ],
      activeTabId: 'tab-1',
    });

    usePdfTabStore.getState().updateActiveTab({ annotations: [ann] });
    expect(usePdfTabStore.getState().canUndo()).toBe(true);

    usePdfTabStore.getState().undo();
    // Wait for the microtask that resets _isUndoRedoOperation
    await new Promise((r) => setTimeout(r, 0));

    const tab = usePdfTabStore.getState().getActiveTab();
    expect(tab?.annotations).toHaveLength(0);
    expect(tab?.historyIndex).toBe(0);
  });

  it('redo restores annotations after undo', async () => {
    const ann = { id: 'ann-1', x: 10, y: 20, pageNumber: 1, type: 'TEXT' as const };
    usePdfTabStore.setState({
      tabs: [
        { id: 'tab-1', filename: 'a.pdf', pdfUrl: '', numPages: 1, annotations: [], bubbleOrder: [], zoom: 'fitWidth', rotation: 0, isDirty: false, history: [{ annotations: [], bubbleOrder: [] }], historyIndex: 0 },
      ],
      activeTabId: 'tab-1',
    });

    usePdfTabStore.getState().updateActiveTab({ annotations: [ann] });
    usePdfTabStore.getState().undo();
    await new Promise((r) => setTimeout(r, 0));

    expect(usePdfTabStore.getState().canRedo()).toBe(true);
    usePdfTabStore.getState().redo();
    await new Promise((r) => setTimeout(r, 0));

    const tab = usePdfTabStore.getState().getActiveTab();
    expect(tab?.annotations).toHaveLength(1);
  });

  it('reorders tabs', () => {
    usePdfTabStore.setState({
      tabs: [
        { id: 'tab-1', filename: 'a.pdf', pdfUrl: '', numPages: 1, annotations: [], bubbleOrder: [], zoom: 'fitWidth', rotation: 0, isDirty: false, history: [], historyIndex: 0 },
        { id: 'tab-2', filename: 'b.pdf', pdfUrl: '', numPages: 1, annotations: [], bubbleOrder: [], zoom: 'fitWidth', rotation: 0, isDirty: false, history: [], historyIndex: 0 },
        { id: 'tab-3', filename: 'c.pdf', pdfUrl: '', numPages: 1, annotations: [], bubbleOrder: [], zoom: 'fitWidth', rotation: 0, isDirty: false, history: [], historyIndex: 0 },
      ],
      activeTabId: 'tab-1',
    });

    usePdfTabStore.getState().reorderTabs(0, 2);
    const ids = usePdfTabStore.getState().tabs.map((t) => t.id);
    expect(ids).toEqual(['tab-2', 'tab-3', 'tab-1']);
  });
});
