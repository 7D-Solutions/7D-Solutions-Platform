// Viewport store — zoom, rotation, and text search state.

import { create } from 'zustand';

export type ZoomLevel = 'fitWidth' | 'fitPage' | number;

interface SearchResult {
  page: number;
  index: number;
  text: string;
}

interface ViewportStore {
  zoom: ZoomLevel;
  rotation: 0 | 90 | 180 | 270;
  searchTerm: string;
  searchResults: SearchResult[];
  currentSearchIndex: number;

  setZoom: (zoom: ZoomLevel) => void;
  rotateClockwise: () => void;
  rotateCounterClockwise: () => void;
  setSearchTerm: (term: string) => void;
  setSearchResults: (results: SearchResult[]) => void;
  nextSearchResult: () => void;
  prevSearchResult: () => void;
  clearSearch: () => void;
}

export const useViewportStore = create<ViewportStore>((set) => ({
  zoom: 'fitWidth',
  rotation: 0,
  searchTerm: '',
  searchResults: [],
  currentSearchIndex: 0,

  setZoom: (zoom) => set({ zoom }),

  rotateClockwise: () =>
    set((state) => ({
      rotation: ((state.rotation + 90) % 360) as 0 | 90 | 180 | 270,
    })),

  rotateCounterClockwise: () =>
    set((state) => ({
      rotation: ((state.rotation - 90 + 360) % 360) as 0 | 90 | 180 | 270,
    })),

  setSearchTerm: (term) => set({ searchTerm: term }),
  setSearchResults: (results) => set({ searchResults: results, currentSearchIndex: 0 }),

  nextSearchResult: () =>
    set((state) => ({
      currentSearchIndex:
        state.searchResults.length > 0
          ? (state.currentSearchIndex + 1) % state.searchResults.length
          : 0,
    })),

  prevSearchResult: () =>
    set((state) => ({
      currentSearchIndex:
        state.searchResults.length > 0
          ? (state.currentSearchIndex - 1 + state.searchResults.length) %
            state.searchResults.length
          : 0,
    })),

  clearSearch: () => set({ searchTerm: '', searchResults: [], currentSearchIndex: 0 }),
}));
