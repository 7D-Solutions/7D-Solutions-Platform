// Smoke tests for the form store.
// Tests local state management. API-dependent actions (loadTemplates, createTemplate, etc.)
// require a running backend — those are verified via the manual smoke test checklist.

import { describe, it, expect, beforeEach } from 'vitest';
import { useFormStore } from '../stores/formStore.ts';

function resetStore() {
  useFormStore.setState({
    templates: [],
    activeTemplate: null,
    fields: [],
    submissions: [],
    activeSubmission: null,
    fieldData: {},
    selectedFieldId: null,
    isLoading: false,
    error: null,
    isSavingDraft: false,
  });
}

describe('formStore', () => {
  beforeEach(resetStore);

  it('starts with empty state', () => {
    const state = useFormStore.getState();
    expect(state.templates).toEqual([]);
    expect(state.activeTemplate).toBeNull();
    expect(state.fields).toEqual([]);
    expect(state.fieldData).toEqual({});
    expect(state.isLoading).toBe(false);
    expect(state.error).toBeNull();
  });

  it('updates individual field data', () => {
    useFormStore.getState().updateFieldData('name', 'John');
    useFormStore.getState().updateFieldData('age', 30);
    expect(useFormStore.getState().fieldData).toEqual({
      name: 'John',
      age: 30,
    });
  });

  it('replaces all field data', () => {
    useFormStore.getState().updateFieldData('name', 'John');
    useFormStore.getState().setFieldData({ email: 'john@example.com' });
    expect(useFormStore.getState().fieldData).toEqual({
      email: 'john@example.com',
    });
  });

  it('selects a field', () => {
    useFormStore.getState().setSelectedFieldId('field-1');
    expect(useFormStore.getState().selectedFieldId).toBe('field-1');

    useFormStore.getState().setSelectedFieldId(null);
    expect(useFormStore.getState().selectedFieldId).toBeNull();
  });

  it('clears form state', () => {
    useFormStore.setState({
      activeTemplate: { id: 't1', tenant_id: 'ten', name: 'T', description: null, created_by: 'u', created_at: '', updated_at: '' },
      fields: [{ id: 'f1', template_id: 't1', field_key: 'k', field_label: 'L', field_type: 'text', validation_rules: {}, pdf_position: { x: 0, y: 0, width: 100, height: 30, page: 1 }, display_order: 0 }],
      fieldData: { k: 'v' },
      selectedFieldId: 'f1',
      error: 'old error',
    });

    useFormStore.getState().clearForm();
    const state = useFormStore.getState();
    expect(state.activeTemplate).toBeNull();
    expect(state.fields).toEqual([]);
    expect(state.fieldData).toEqual({});
    expect(state.selectedFieldId).toBeNull();
    expect(state.error).toBeNull();
  });

  it('clears error', () => {
    useFormStore.setState({ error: 'Something went wrong' });
    useFormStore.getState().clearError();
    expect(useFormStore.getState().error).toBeNull();
  });
});
