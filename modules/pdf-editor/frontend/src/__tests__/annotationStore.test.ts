// Smoke tests for the annotation store.
// These test pure browser-local state management (no API calls).

import { describe, it, expect, beforeEach } from 'vitest';
import { useAnnotationStore } from '../stores/annotationStore.ts';

function resetStore() {
  useAnnotationStore.setState({
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
  });
}

describe('annotationStore', () => {
  beforeEach(resetStore);

  it('starts with no selected type', () => {
    const state = useAnnotationStore.getState();
    expect(state.selectedType).toBeNull();
    expect(state.stampType).toBe('APPROVED');
  });

  it('sets selected annotation type', () => {
    useAnnotationStore.getState().setSelectedType('ARROW');
    expect(useAnnotationStore.getState().selectedType).toBe('ARROW');
  });

  it('sets stamp type', () => {
    useAnnotationStore.getState().setStampType('REJECTED');
    expect(useAnnotationStore.getState().stampType).toBe('REJECTED');
  });

  it('manages dragged annotation state', () => {
    useAnnotationStore.getState().setDraggedAnnotation('ann-1');
    expect(useAnnotationStore.getState().draggedAnnotation).toBe('ann-1');

    useAnnotationStore.getState().setDraggedAnnotation(null);
    expect(useAnnotationStore.getState().draggedAnnotation).toBeNull();
  });

  it('manages selected annotation', () => {
    useAnnotationStore.getState().setSelectedAnnotation('ann-2');
    expect(useAnnotationStore.getState().selectedAnnotation).toBe('ann-2');
  });

  it('toggles edit mode', () => {
    expect(useAnnotationStore.getState().isEditMode).toBe(false);
    useAnnotationStore.getState().setIsEditMode(true);
    expect(useAnnotationStore.getState().isEditMode).toBe(true);
  });

  it('manages text position', () => {
    const pos = { x: 100, y: 200, pageNumber: 1 };
    useAnnotationStore.getState().setNewTextPosition(pos);
    expect(useAnnotationStore.getState().newTextPosition).toEqual(pos);
  });

  it('tracks skip delete confirmation preference', () => {
    useAnnotationStore.getState().setSkipDeleteConfirmation(true);
    expect(useAnnotationStore.getState().skipDeleteConfirmation).toBe(true);
  });
});
