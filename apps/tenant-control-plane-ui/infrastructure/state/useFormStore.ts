// ============================================================
// Form state store factory — tab-scoped, localStorage-persisted
// Port from: docs/reference/fireproof/src/infrastructure/state/useFormStore.ts
// Adapted: uses next/navigation instead of react-router-dom
// ============================================================
'use client';
import { create } from 'zustand';
import type { UseBoundStore, StoreApi } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import { useEffect } from 'react';
import { usePathname } from 'next/navigation';
import { useActiveTabId, useTabStore } from './tabStore';
import { useBeforeUnload } from '../hooks/useBeforeUnload';

interface FormStoreState<T> {
  formData: T;
  formErrors: Record<string, string>;
  isDirty: boolean;
  hasUnsavedChanges: boolean;
  changedFields: Set<string>;
  savedFormData: T | null;

  updateField: <K extends keyof T>(field: K, value: T[K]) => void;
  updateFields: (updates: Partial<T>) => void;
  setError: (field: string, error: string) => void;
  clearError: (field: string) => void;
  clearAllErrors: () => void;
  setFormData: (data: T) => void;
  resetForm: () => void;
  markAsSaved: () => void;
  getChangedFieldsWithValues: () => Array<{ field: string; oldValue: unknown; newValue: unknown }>;
}

const storeCache = new Map<string, UseBoundStore<StoreApi<unknown>>>();

/**
 * Tab-scoped, persistent form state factory.
 * Form data survives tab switches. Dirty-state syncs to the tab title.
 * Browser close warning fires when hasUnsavedChanges is true.
 *
 * @example
 * const { formData, updateField, isDirty } = useFormStore('tenant-settings', {
 *   planId: '', connectionId: ''
 * });
 */
export function useFormStore<T extends Record<string, unknown>>(
  formKey: string,
  initialData: T
) {
  const pathname = usePathname();
  const activeTabId = useActiveTabId();
  const storageKey = `form-${formKey}-${activeTabId}`;

  let store: UseBoundStore<StoreApi<FormStoreState<T>>>;
  if (storeCache.has(storageKey)) {
    store = storeCache.get(storageKey) as UseBoundStore<StoreApi<FormStoreState<T>>>;
  } else {
    store = create<FormStoreState<T>>()(
      persist(
        (set, get) => ({
          formData: initialData,
          formErrors: {},
          isDirty: false,
          hasUnsavedChanges: false,
          changedFields: new Set<string>(),
          savedFormData: null,

          updateField: (field, value) => {
            set((state) => {
              const savedData = state.savedFormData || initialData;
              const fieldKey = String(field);
              const newChangedFields = new Set(state.changedFields);
              if (JSON.stringify(value) !== JSON.stringify(savedData[field])) {
                newChangedFields.add(fieldKey);
              } else {
                newChangedFields.delete(fieldKey);
              }
              return {
                formData: { ...state.formData, [field]: value },
                isDirty: true,
                hasUnsavedChanges: newChangedFields.size > 0,
                changedFields: newChangedFields,
              };
            });
          },

          updateFields: (updates) => {
            set((state) => {
              const savedData = state.savedFormData || initialData;
              const newChangedFields = new Set(state.changedFields);
              Object.entries(updates).forEach(([key, value]) => {
                if (JSON.stringify(value) !== JSON.stringify(savedData[key as keyof T])) {
                  newChangedFields.add(key);
                } else {
                  newChangedFields.delete(key);
                }
              });
              return {
                formData: { ...state.formData, ...updates },
                isDirty: true,
                hasUnsavedChanges: newChangedFields.size > 0,
                changedFields: newChangedFields,
              };
            });
          },

          setError: (field, error) => set((state) => ({
            formErrors: { ...state.formErrors, [field]: error },
          })),

          clearError: (field) => set((state) => {
            const { [field]: _removed, ...rest } = state.formErrors;
            return { formErrors: rest };
          }),

          clearAllErrors: () => set({ formErrors: {} }),

          setFormData: (data) => set({
            formData: data, isDirty: false, hasUnsavedChanges: false,
            savedFormData: data, changedFields: new Set(),
          }),

          resetForm: () => set({
            formData: initialData, formErrors: {}, isDirty: false,
            hasUnsavedChanges: false, changedFields: new Set(), savedFormData: null,
          }),

          markAsSaved: () => set((state) => ({
            hasUnsavedChanges: false, changedFields: new Set(), savedFormData: state.formData,
          })),

          getChangedFieldsWithValues: () => {
            const state = get();
            const savedData = state.savedFormData || initialData;
            const changes: Array<{ field: string; oldValue: unknown; newValue: unknown }> = [];
            state.changedFields.forEach((field) => {
              changes.push({
                field,
                oldValue: savedData[field as keyof T],
                newValue: state.formData[field as keyof T],
              });
            });
            return changes;
          },
        }),
        {
          name: storageKey,
          storage: createJSONStorage(() => localStorage),
          partialize: (state) => ({
            formData: state.formData,
            isDirty: state.isDirty,
            hasUnsavedChanges: state.hasUnsavedChanges,
            savedFormData: state.savedFormData,
          }),
        }
      )
    );
    storeCache.set(storageKey, store);
  }

  const formState = store();

  useEffect(() => {
    const tabStore = useTabStore.getState();
    const currentTab = tabStore.findTabByRoute(pathname ?? '/');
    if (currentTab) {
      tabStore.updateTab(currentTab.id, { isDirty: formState.hasUnsavedChanges });
    }
  }, [formState.hasUnsavedChanges, pathname]);

  useBeforeUnload(formState.hasUnsavedChanges);

  return formState;
}
