// Smoke tests for toolbar store.

import { describe, it, expect, beforeEach } from 'vitest';
import { useToolbarStore } from '../stores/toolbarStore.ts';

function resetStore() {
  useToolbarStore.setState({
    activeButtons: ['save', 'undo', 'redo', 'print'],
    isCustomizing: false,
  });
}

describe('toolbarStore', () => {
  beforeEach(resetStore);

  it('starts with default buttons', () => {
    const state = useToolbarStore.getState();
    expect(state.activeButtons).toEqual(['save', 'undo', 'redo', 'print']);
  });

  it('adds a button', () => {
    useToolbarStore.getState().addButton('export');
    expect(useToolbarStore.getState().activeButtons).toContain('export');
  });

  it('does not duplicate an existing button', () => {
    useToolbarStore.getState().addButton('save');
    expect(useToolbarStore.getState().activeButtons.filter((b) => b === 'save')).toHaveLength(1);
  });

  it('removes a button', () => {
    useToolbarStore.getState().removeButton('print');
    expect(useToolbarStore.getState().activeButtons).not.toContain('print');
  });

  it('reorders buttons', () => {
    useToolbarStore.getState().reorderButtons(0, 2);
    expect(useToolbarStore.getState().activeButtons).toEqual(['undo', 'redo', 'save', 'print']);
  });

  it('resets to defaults', () => {
    useToolbarStore.getState().addButton('export');
    useToolbarStore.getState().resetToDefaults();
    expect(useToolbarStore.getState().activeButtons).toEqual(['save', 'undo', 'redo', 'print']);
  });

  it('looks up button config', () => {
    const config = useToolbarStore.getState().getButtonConfig('save');
    expect(config?.label).toBe('Save');
    expect(config?.shortcut).toBe('Ctrl+S');
  });

  it('returns undefined for unknown button', () => {
    const config = useToolbarStore.getState().getButtonConfig('nonexistent' as any);
    expect(config).toBeUndefined();
  });
});
