// Smoke tests for viewport store.

import { describe, it, expect, beforeEach } from 'vitest';
import { useViewportStore } from '../stores/viewportStore.ts';

function resetStore() {
  useViewportStore.setState({
    zoom: 'fitWidth',
    rotation: 0,
    searchTerm: '',
    searchResults: [],
    currentSearchIndex: 0,
  });
}

describe('viewportStore', () => {
  beforeEach(resetStore);

  it('starts at fitWidth zoom with no rotation', () => {
    const state = useViewportStore.getState();
    expect(state.zoom).toBe('fitWidth');
    expect(state.rotation).toBe(0);
  });

  it('sets zoom level', () => {
    useViewportStore.getState().setZoom(150);
    expect(useViewportStore.getState().zoom).toBe(150);
  });

  it('rotates clockwise', () => {
    useViewportStore.getState().rotateClockwise();
    expect(useViewportStore.getState().rotation).toBe(90);
    useViewportStore.getState().rotateClockwise();
    expect(useViewportStore.getState().rotation).toBe(180);
    useViewportStore.getState().rotateClockwise();
    expect(useViewportStore.getState().rotation).toBe(270);
    useViewportStore.getState().rotateClockwise();
    expect(useViewportStore.getState().rotation).toBe(0);
  });

  it('rotates counter-clockwise', () => {
    useViewportStore.getState().rotateCounterClockwise();
    expect(useViewportStore.getState().rotation).toBe(270);
  });

  it('manages search results', () => {
    const results = [
      { page: 1, index: 0, text: 'foo' },
      { page: 2, index: 1, text: 'bar' },
    ];
    useViewportStore.getState().setSearchTerm('test');
    useViewportStore.getState().setSearchResults(results);

    expect(useViewportStore.getState().searchResults).toHaveLength(2);
    expect(useViewportStore.getState().currentSearchIndex).toBe(0);

    useViewportStore.getState().nextSearchResult();
    expect(useViewportStore.getState().currentSearchIndex).toBe(1);

    useViewportStore.getState().nextSearchResult();
    expect(useViewportStore.getState().currentSearchIndex).toBe(0); // wraps

    useViewportStore.getState().prevSearchResult();
    expect(useViewportStore.getState().currentSearchIndex).toBe(1); // wraps back
  });

  it('clears search', () => {
    useViewportStore.getState().setSearchTerm('test');
    useViewportStore.getState().setSearchResults([{ page: 1, index: 0, text: 'test' }]);
    useViewportStore.getState().clearSearch();

    const state = useViewportStore.getState();
    expect(state.searchTerm).toBe('');
    expect(state.searchResults).toEqual([]);
    expect(state.currentSearchIndex).toBe(0);
  });
});
