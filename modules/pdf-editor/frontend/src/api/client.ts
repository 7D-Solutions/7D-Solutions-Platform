// Typed API client for the stateless PDF Editor backend.
//
// Base URL comes from Vite env: VITE_PDF_API_BASE_URL
// Auth token is set via pdfApi.setAuthToken() and sent as Bearer header.

import type {
  Annotation,
  ApiError,
  AutosaveRequest,
  CreateFieldRequest,
  CreateSubmissionRequest,
  CreateTemplateRequest,
  FormField,
  FormSubmission,
  FormTemplate,
  ListSubmissionsParams,
  ListTemplatesParams,
  ReorderFieldsRequest,
  UpdateFieldRequest,
  UpdateTemplateRequest,
} from './types.ts';

const BASE_URL: string =
  import.meta.env?.VITE_PDF_API_BASE_URL ?? 'http://localhost:3100';

let authToken: string | null = null;

function headers(extra?: Record<string, string>): Record<string, string> {
  const h: Record<string, string> = { ...extra };
  if (authToken) {
    h['Authorization'] = `Bearer ${authToken}`;
  }
  return h;
}

function qs(params: Record<string, string | number | boolean | null | undefined>): string {
  const entries: [string, string][] = [];
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null) {
      entries.push([k, String(v)]);
    }
  }
  if (entries.length === 0) return '';
  return '?' + new URLSearchParams(entries).toString();
}

class PdfApiError extends Error {
  status: number;
  body: ApiError;

  constructor(status: number, body: ApiError) {
    super(body.message ?? body.error);
    this.name = 'PdfApiError';
    this.status = status;
    this.body = body;
  }
}

async function jsonOrThrow<T>(res: Response): Promise<T> {
  if (!res.ok) {
    let body: ApiError;
    try {
      body = await res.json() as ApiError;
    } catch {
      body = { error: res.statusText };
    }
    throw new PdfApiError(res.status, body);
  }
  return res.json() as Promise<T>;
}

async function blobOrThrow(res: Response): Promise<Blob> {
  if (!res.ok) {
    let body: ApiError;
    try {
      body = await res.json() as ApiError;
    } catch {
      body = { error: res.statusText };
    }
    throw new PdfApiError(res.status, body);
  }
  return res.blob();
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export const pdfApi = {
  /** Set the JWT auth token for all subsequent requests. */
  setAuthToken(token: string | null): void {
    authToken = token;
  },

  // =========================================================================
  // PDF Processing (stateless)
  // =========================================================================

  /**
   * Render annotations onto a PDF.
   *
   * Sends PDF bytes + annotation JSON as multipart, returns annotated PDF.
   */
  async renderAnnotations(
    file: File | Blob,
    annotations: Annotation[],
  ): Promise<Blob> {
    const form = new FormData();
    form.append('file', file);
    form.append('annotations', JSON.stringify(annotations));

    const res = await fetch(`${BASE_URL}/api/pdf/render-annotations`, {
      method: 'POST',
      headers: headers(),
      body: form,
    });

    return blobOrThrow(res);
  },

  /**
   * Generate a filled PDF from a submission.
   *
   * Sends PDF template bytes + submission ID, returns filled PDF.
   */
  async generatePdf(
    file: File | Blob,
    submissionId: string,
    tenantId: string,
  ): Promise<Blob> {
    const form = new FormData();
    form.append('file', file);

    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/submissions/${submissionId}/generate${qs({ tenant_id: tenantId })}`,
      {
        method: 'POST',
        headers: headers(),
        body: form,
      },
    );

    return blobOrThrow(res);
  },

  // =========================================================================
  // Form Templates
  // =========================================================================

  async createTemplate(req: CreateTemplateRequest): Promise<FormTemplate> {
    const res = await fetch(`${BASE_URL}/api/pdf/forms/templates`, {
      method: 'POST',
      headers: headers({ 'Content-Type': 'application/json' }),
      body: JSON.stringify(req),
    });
    return jsonOrThrow<FormTemplate>(res);
  },

  async listTemplates(params: ListTemplatesParams): Promise<FormTemplate[]> {
    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/templates${qs({ ...params })}`,
      { headers: headers() },
    );
    return jsonOrThrow<FormTemplate[]>(res);
  },

  async getTemplate(id: string, tenantId: string): Promise<FormTemplate> {
    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/templates/${id}${qs({ tenant_id: tenantId })}`,
      { headers: headers() },
    );
    return jsonOrThrow<FormTemplate>(res);
  },

  async updateTemplate(
    id: string,
    tenantId: string,
    req: UpdateTemplateRequest,
  ): Promise<FormTemplate> {
    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/templates/${id}${qs({ tenant_id: tenantId })}`,
      {
        method: 'PUT',
        headers: headers({ 'Content-Type': 'application/json' }),
        body: JSON.stringify(req),
      },
    );
    return jsonOrThrow<FormTemplate>(res);
  },

  // =========================================================================
  // Form Fields
  // =========================================================================

  async createField(
    templateId: string,
    tenantId: string,
    req: CreateFieldRequest,
  ): Promise<FormField> {
    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/templates/${templateId}/fields${qs({ tenant_id: tenantId })}`,
      {
        method: 'POST',
        headers: headers({ 'Content-Type': 'application/json' }),
        body: JSON.stringify(req),
      },
    );
    return jsonOrThrow<FormField>(res);
  },

  async listFields(
    templateId: string,
    tenantId: string,
  ): Promise<FormField[]> {
    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/templates/${templateId}/fields${qs({ tenant_id: tenantId })}`,
      { headers: headers() },
    );
    return jsonOrThrow<FormField[]>(res);
  },

  async updateField(
    templateId: string,
    fieldId: string,
    tenantId: string,
    req: UpdateFieldRequest,
  ): Promise<FormField> {
    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/templates/${templateId}/fields/${fieldId}${qs({ tenant_id: tenantId })}`,
      {
        method: 'PUT',
        headers: headers({ 'Content-Type': 'application/json' }),
        body: JSON.stringify(req),
      },
    );
    return jsonOrThrow<FormField>(res);
  },

  async reorderFields(
    templateId: string,
    tenantId: string,
    req: ReorderFieldsRequest,
  ): Promise<FormField[]> {
    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/templates/${templateId}/fields/reorder${qs({ tenant_id: tenantId })}`,
      {
        method: 'POST',
        headers: headers({ 'Content-Type': 'application/json' }),
        body: JSON.stringify(req),
      },
    );
    return jsonOrThrow<FormField[]>(res);
  },

  // =========================================================================
  // Form Submissions
  // =========================================================================

  async createSubmission(
    req: CreateSubmissionRequest,
  ): Promise<FormSubmission> {
    const res = await fetch(`${BASE_URL}/api/pdf/forms/submissions`, {
      method: 'POST',
      headers: headers({ 'Content-Type': 'application/json' }),
      body: JSON.stringify(req),
    });
    return jsonOrThrow<FormSubmission>(res);
  },

  async getSubmission(
    id: string,
    tenantId: string,
  ): Promise<FormSubmission> {
    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/submissions/${id}${qs({ tenant_id: tenantId })}`,
      { headers: headers() },
    );
    return jsonOrThrow<FormSubmission>(res);
  },

  async listSubmissions(
    params: ListSubmissionsParams,
  ): Promise<FormSubmission[]> {
    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/submissions${qs({ ...params })}`,
      { headers: headers() },
    );
    return jsonOrThrow<FormSubmission[]>(res);
  },

  async autosaveSubmission(
    id: string,
    tenantId: string,
    req: AutosaveRequest,
  ): Promise<FormSubmission> {
    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/submissions/${id}${qs({ tenant_id: tenantId })}`,
      {
        method: 'PUT',
        headers: headers({ 'Content-Type': 'application/json' }),
        body: JSON.stringify(req),
      },
    );
    return jsonOrThrow<FormSubmission>(res);
  },

  async submitSubmission(
    id: string,
    tenantId: string,
  ): Promise<FormSubmission> {
    const res = await fetch(
      `${BASE_URL}/api/pdf/forms/submissions/${id}/submit${qs({ tenant_id: tenantId })}`,
      {
        method: 'POST',
        headers: headers(),
      },
    );
    return jsonOrThrow<FormSubmission>(res);
  },
};

export { PdfApiError };
export type { ApiError };
