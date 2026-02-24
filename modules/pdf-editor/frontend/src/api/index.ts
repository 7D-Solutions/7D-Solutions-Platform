// Public API surface for the PDF Editor typed client.
//
// Consumers: import from this file, not from client.ts or types.ts directly.
//
// Usage:
//   import { pdfApi, PdfApiError } from './api';
//   import type { Annotation, FormTemplate, FormField, FormSubmission } from './api';

export { pdfApi, PdfApiError } from './client.ts';
export type { ApiError } from './client.ts';

export type {
  // Annotation types
  Annotation,
  AnnotationType,
  StampType,
  ShapeType,
  TextRect,
  Point,
  SignaturePoint,

  // Form templates
  FormTemplate,
  CreateTemplateRequest,
  UpdateTemplateRequest,
  ListTemplatesParams,

  // Form fields
  FormField,
  FieldType,
  PdfPosition,
  CreateFieldRequest,
  UpdateFieldRequest,
  ReorderFieldsRequest,

  // Form submissions
  FormSubmission,
  SubmissionStatus,
  CreateSubmissionRequest,
  AutosaveRequest,
  ListSubmissionsParams,
} from './types.ts';
