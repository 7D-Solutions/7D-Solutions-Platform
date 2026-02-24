// Annotation UI state store.
// Annotations live in the browser only (per-tab in pdfTabStore).
// No server-side annotation persistence — the render-annotations endpoint
// is a stateless "burn and return" operation.

import { create } from 'zustand';
import type { AnnotationType, StampType } from '../api/types.ts';
import { pdfApi } from '../api/client.ts';

interface AnnotationStore {
  // Tool selection
  selectedType: AnnotationType | null;
  stampType: StampType;
  customText: string;

  // Annotation interactions
  draggedAnnotation: string | null;
  selectedAnnotation: string | null;
  editingAnnotation: string | null;

  // Text annotation mode
  isAddingText: boolean;
  newTextPosition: { x: number; y: number; pageNumber: number } | null;

  // Edit mode
  isEditMode: boolean;

  // Bubble leader line
  addingLeaderForBubble: string | null;

  // Session preferences
  skipDeleteConfirmation: boolean;

  // Render state
  isRendering: boolean;
  renderError: string | null;

  // Tool selection actions
  setSelectedType: (type: AnnotationType | null) => void;
  setStampType: (stampType: StampType) => void;
  setCustomText: (text: string) => void;

  // Interaction actions
  setDraggedAnnotation: (id: string | null) => void;
  setSelectedAnnotation: (id: string | null) => void;
  setEditingAnnotation: (id: string | null) => void;

  // Text mode actions
  setIsAddingText: (isAdding: boolean) => void;
  setNewTextPosition: (position: { x: number; y: number; pageNumber: number } | null) => void;

  // Edit mode actions
  setIsEditMode: (isEditMode: boolean) => void;

  // Bubble leader actions
  setAddingLeaderForBubble: (bubbleId: string | null) => void;

  // Session preference actions
  setSkipDeleteConfirmation: (skip: boolean) => void;

  // Render action: send PDF bytes + annotations, get back burned PDF
  renderAnnotations: (
    file: File | Blob,
    annotations: import('../api/types.ts').Annotation[],
  ) => Promise<Blob | null>;
}

export const useAnnotationStore = create<AnnotationStore>((set) => ({
  selectedType: null,
  stampType: 'APPROVED',
  customText: '',
  draggedAnnotation: null,
  selectedAnnotation: null,
  editingAnnotation: null,
  isAddingText: false,
  newTextPosition: null,
  isEditMode: false,
  addingLeaderForBubble: null,
  skipDeleteConfirmation: false,
  isRendering: false,
  renderError: null,

  setSelectedType: (type) => set({ selectedType: type }),
  setStampType: (stampType) => set({ stampType }),
  setCustomText: (text) => set({ customText: text }),

  setDraggedAnnotation: (id) => set({ draggedAnnotation: id }),
  setSelectedAnnotation: (id) => set({ selectedAnnotation: id }),
  setEditingAnnotation: (id) => set({ editingAnnotation: id }),

  setIsAddingText: (isAdding) => set({ isAddingText: isAdding }),
  setNewTextPosition: (position) => set({ newTextPosition: position }),

  setIsEditMode: (isEditMode) => set({ isEditMode }),

  setAddingLeaderForBubble: (bubbleId) => set({ addingLeaderForBubble: bubbleId }),

  setSkipDeleteConfirmation: (skip) => set({ skipDeleteConfirmation: skip }),

  renderAnnotations: async (file, annotations) => {
    set({ isRendering: true, renderError: null });
    try {
      const result = await pdfApi.renderAnnotations(file, annotations);
      set({ isRendering: false });
      return result;
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Render failed';
      set({ isRendering: false, renderError: message });
      return null;
    }
  },
}));
