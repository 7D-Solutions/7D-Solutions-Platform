// Smoke tests for the UI store.
// Tests browser-local PDF state management.

import { describe, it, expect, beforeEach } from 'vitest';
import { useUIStore } from '../stores/uiStore.ts';

function resetStore() {
  useUIStore.setState({
    pdfFile: null,
    pdfUrl: '',
    numPages: 0,
    mode: 'ANNOTATE',
    isProcessing: false,
    username: '',
    showPreview: false,
    previewUrl: '',
    previewFilename: '',
  });
}

describe('uiStore', () => {
  beforeEach(resetStore);

  it('starts in ANNOTATE mode with no PDF', () => {
    const state = useUIStore.getState();
    expect(state.mode).toBe('ANNOTATE');
    expect(state.pdfFile).toBeNull();
    expect(state.pdfUrl).toBe('');
  });

  it('switches mode to FORM', () => {
    useUIStore.getState().setMode('FORM');
    expect(useUIStore.getState().mode).toBe('FORM');
  });

  it('tracks processing state', () => {
    useUIStore.getState().setIsProcessing(true);
    expect(useUIStore.getState().isProcessing).toBe(true);
  });

  it('sets username', () => {
    useUIStore.getState().setUsername('testuser');
    expect(useUIStore.getState().username).toBe('testuser');
  });

  it('manages preview state', () => {
    useUIStore.getState().showPdfPreview('blob:preview', 'result.pdf');
    const state = useUIStore.getState();
    expect(state.showPreview).toBe(true);
    expect(state.previewUrl).toBe('blob:preview');
    expect(state.previewFilename).toBe('result.pdf');

    useUIStore.getState().hidePreview();
    expect(useUIStore.getState().showPreview).toBe(false);
    expect(useUIStore.getState().previewUrl).toBe('');
  });

  it('resets PDF state', () => {
    useUIStore.getState().setPdfUrl('blob:test');
    useUIStore.getState().setNumPages(5);
    useUIStore.getState().resetPdf();

    const state = useUIStore.getState();
    expect(state.pdfFile).toBeNull();
    expect(state.pdfUrl).toBe('');
    expect(state.numPages).toBe(0);
  });
});
