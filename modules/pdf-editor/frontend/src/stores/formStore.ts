// Form store — uses the typed API client for template/submission CRUD.
// Templates and submissions persist in Postgres via the Rust backend.
// Drafts auto-save via PUT /submissions/:id.

import { create } from 'zustand';
import { pdfApi, PdfApiError } from '../api/client.ts';
import type {
  FormTemplate,
  FormField,
  FormSubmission,
  CreateTemplateRequest,
  UpdateTemplateRequest,
  CreateFieldRequest,
  UpdateFieldRequest,
  CreateSubmissionRequest,
} from '../api/types.ts';

interface FormStore {
  // Template state
  templates: FormTemplate[];
  activeTemplate: FormTemplate | null;
  fields: FormField[];

  // Submission state
  submissions: FormSubmission[];
  activeSubmission: FormSubmission | null;
  fieldData: Record<string, unknown>;

  // UI state
  selectedFieldId: string | null;
  isLoading: boolean;
  error: string | null;
  isSavingDraft: boolean;

  // Template actions
  loadTemplates: (tenantId: string) => Promise<void>;
  loadTemplate: (id: string, tenantId: string) => Promise<void>;
  createTemplate: (req: CreateTemplateRequest) => Promise<FormTemplate | null>;
  updateTemplate: (id: string, tenantId: string, req: UpdateTemplateRequest) => Promise<void>;

  // Field actions
  loadFields: (templateId: string, tenantId: string) => Promise<void>;
  createField: (templateId: string, tenantId: string, req: CreateFieldRequest) => Promise<FormField | null>;
  updateField: (templateId: string, fieldId: string, tenantId: string, req: UpdateFieldRequest) => Promise<void>;

  // Submission actions
  loadSubmissions: (tenantId: string, templateId?: string) => Promise<void>;
  createSubmission: (req: CreateSubmissionRequest) => Promise<FormSubmission | null>;
  autosaveDraft: (id: string, tenantId: string) => Promise<void>;
  submitSubmission: (id: string, tenantId: string) => Promise<void>;
  generatePdf: (file: File | Blob, submissionId: string, tenantId: string) => Promise<Blob | null>;

  // Local field data
  updateFieldData: (key: string, value: unknown) => void;
  setFieldData: (data: Record<string, unknown>) => void;
  setSelectedFieldId: (id: string | null) => void;

  // Reset
  clearForm: () => void;
  clearError: () => void;
}

function extractError(err: unknown): string {
  if (err instanceof PdfApiError) return err.body.message ?? err.body.error;
  if (err instanceof Error) return err.message;
  return 'Unknown error';
}

export const useFormStore = create<FormStore>((set, get) => ({
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

  // -- Template actions --

  loadTemplates: async (tenantId) => {
    set({ isLoading: true, error: null });
    try {
      const templates = await pdfApi.listTemplates({ tenant_id: tenantId });
      set({ templates, isLoading: false });
    } catch (err) {
      set({ isLoading: false, error: extractError(err) });
    }
  },

  loadTemplate: async (id, tenantId) => {
    set({ isLoading: true, error: null });
    try {
      const template = await pdfApi.getTemplate(id, tenantId);
      set({ activeTemplate: template, isLoading: false });
    } catch (err) {
      set({ isLoading: false, error: extractError(err) });
    }
  },

  createTemplate: async (req) => {
    set({ isLoading: true, error: null });
    try {
      const template = await pdfApi.createTemplate(req);
      set((s) => ({
        templates: [...s.templates, template],
        activeTemplate: template,
        isLoading: false,
      }));
      return template;
    } catch (err) {
      set({ isLoading: false, error: extractError(err) });
      return null;
    }
  },

  updateTemplate: async (id, tenantId, req) => {
    set({ isLoading: true, error: null });
    try {
      const updated = await pdfApi.updateTemplate(id, tenantId, req);
      set((s) => ({
        templates: s.templates.map((t) => (t.id === id ? updated : t)),
        activeTemplate: s.activeTemplate?.id === id ? updated : s.activeTemplate,
        isLoading: false,
      }));
    } catch (err) {
      set({ isLoading: false, error: extractError(err) });
    }
  },

  // -- Field actions --

  loadFields: async (templateId, tenantId) => {
    set({ isLoading: true, error: null });
    try {
      const fields = await pdfApi.listFields(templateId, tenantId);
      set({ fields, isLoading: false });
    } catch (err) {
      set({ isLoading: false, error: extractError(err) });
    }
  },

  createField: async (templateId, tenantId, req) => {
    set({ isLoading: true, error: null });
    try {
      const field = await pdfApi.createField(templateId, tenantId, req);
      set((s) => ({ fields: [...s.fields, field], isLoading: false }));
      return field;
    } catch (err) {
      set({ isLoading: false, error: extractError(err) });
      return null;
    }
  },

  updateField: async (templateId, fieldId, tenantId, req) => {
    set({ isLoading: true, error: null });
    try {
      const updated = await pdfApi.updateField(templateId, fieldId, tenantId, req);
      set((s) => ({
        fields: s.fields.map((f) => (f.id === fieldId ? updated : f)),
        isLoading: false,
      }));
    } catch (err) {
      set({ isLoading: false, error: extractError(err) });
    }
  },

  // -- Submission actions --

  loadSubmissions: async (tenantId, templateId) => {
    set({ isLoading: true, error: null });
    try {
      const submissions = await pdfApi.listSubmissions({
        tenant_id: tenantId,
        template_id: templateId,
      });
      set({ submissions, isLoading: false });
    } catch (err) {
      set({ isLoading: false, error: extractError(err) });
    }
  },

  createSubmission: async (req) => {
    set({ isLoading: true, error: null });
    try {
      const submission = await pdfApi.createSubmission(req);
      set((s) => ({
        submissions: [...s.submissions, submission],
        activeSubmission: submission,
        fieldData: submission.field_data,
        isLoading: false,
      }));
      return submission;
    } catch (err) {
      set({ isLoading: false, error: extractError(err) });
      return null;
    }
  },

  autosaveDraft: async (id, tenantId) => {
    const { fieldData } = get();
    set({ isSavingDraft: true });
    try {
      const updated = await pdfApi.autosaveSubmission(id, tenantId, {
        field_data: fieldData,
      });
      set((s) => ({
        submissions: s.submissions.map((sub) => (sub.id === id ? updated : sub)),
        activeSubmission: s.activeSubmission?.id === id ? updated : s.activeSubmission,
        isSavingDraft: false,
      }));
    } catch (err) {
      set({ isSavingDraft: false, error: extractError(err) });
    }
  },

  submitSubmission: async (id, tenantId) => {
    set({ isLoading: true, error: null });
    try {
      const submitted = await pdfApi.submitSubmission(id, tenantId);
      set((s) => ({
        submissions: s.submissions.map((sub) => (sub.id === id ? submitted : sub)),
        activeSubmission: s.activeSubmission?.id === id ? submitted : s.activeSubmission,
        isLoading: false,
      }));
    } catch (err) {
      set({ isLoading: false, error: extractError(err) });
    }
  },

  generatePdf: async (file, submissionId, tenantId) => {
    set({ isLoading: true, error: null });
    try {
      const blob = await pdfApi.generatePdf(file, submissionId, tenantId);
      set({ isLoading: false });
      return blob;
    } catch (err) {
      set({ isLoading: false, error: extractError(err) });
      return null;
    }
  },

  // -- Local field data --

  updateFieldData: (key, value) =>
    set((s) => ({ fieldData: { ...s.fieldData, [key]: value } })),

  setFieldData: (data) => set({ fieldData: data }),

  setSelectedFieldId: (id) => set({ selectedFieldId: id }),

  clearForm: () =>
    set({
      activeTemplate: null,
      fields: [],
      activeSubmission: null,
      fieldData: {},
      selectedFieldId: null,
      error: null,
    }),

  clearError: () => set({ error: null }),
}));
