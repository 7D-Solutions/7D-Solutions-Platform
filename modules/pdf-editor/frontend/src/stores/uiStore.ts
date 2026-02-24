// UI store — PDF file state, mode, processing status.
// In standalone mode the user opens a PDF from their computer (File object).
// No server-side upload — the file stays in the browser.

import { create } from 'zustand';

export type Mode = 'ANNOTATE' | 'FORM';

interface UIStore {
  // PDF state (browser-local)
  pdfFile: File | null;
  pdfUrl: string;
  numPages: number;

  // UI state
  mode: Mode;
  isProcessing: boolean;
  username: string;

  // Preview state (for rendered/generated PDFs)
  showPreview: boolean;
  previewUrl: string;
  previewFilename: string;

  // Actions
  setPdfFile: (file: File | null) => void;
  openLocalPdf: (file: File) => void;
  setPdfUrl: (url: string) => void;
  setNumPages: (numPages: number) => void;
  setMode: (mode: Mode) => void;
  setIsProcessing: (isProcessing: boolean) => void;
  setUsername: (username: string) => void;

  // Preview actions
  showPdfPreview: (url: string, filename: string) => void;
  hidePreview: () => void;

  // Reset
  resetPdf: () => void;
}

export const useUIStore = create<UIStore>((set) => ({
  pdfFile: null,
  pdfUrl: '',
  numPages: 0,
  mode: 'ANNOTATE',
  isProcessing: false,
  username: '',
  showPreview: false,
  previewUrl: '',
  previewFilename: '',

  setPdfFile: (file) => set({ pdfFile: file }),

  openLocalPdf: (file) => {
    const url = URL.createObjectURL(file);
    set({ pdfFile: file, pdfUrl: url, numPages: 0 });
  },

  setPdfUrl: (url) => set({ pdfUrl: url }),
  setNumPages: (numPages) => set({ numPages }),
  setMode: (mode) => set({ mode }),
  setIsProcessing: (isProcessing) => set({ isProcessing }),
  setUsername: (username) => set({ username }),

  showPdfPreview: (url, filename) =>
    set({ showPreview: true, previewUrl: url, previewFilename: filename }),

  hidePreview: () =>
    set({ showPreview: false, previewUrl: '', previewFilename: '' }),

  resetPdf: () =>
    set({
      pdfFile: null,
      pdfUrl: '',
      numPages: 0,
    }),
}));
