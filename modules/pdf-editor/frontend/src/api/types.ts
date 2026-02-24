// Shared TypeScript types for the PDF Editor API.
// These mirror the Rust backend domain models (modules/pdf-editor/src/domain/).

// ---------------------------------------------------------------------------
// Annotation types (match Rust domain::annotations::types)
// ---------------------------------------------------------------------------

export type AnnotationType =
  | 'CALLOUT'
  | 'ARROW'
  | 'HIGHLIGHT'
  | 'STAMP'
  | 'SHAPE'
  | 'FREEHAND'
  | 'TEXT'
  | 'BUBBLE'
  | 'SIGNATURE';

export type StampType =
  | 'APPROVED'
  | 'REJECTED'
  | 'FAI_REQUIRED'
  | 'HOLD'
  | 'REVIEWED'
  | 'VERIFIED'
  | 'CUSTOM';

export type ShapeType =
  | 'RECTANGLE'
  | 'CIRCLE'
  | 'LINE'
  | 'POLYGON'
  | 'REVISION_CLOUD';

export interface TextRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Point {
  x: number;
  y: number;
}

export interface SignaturePoint {
  x: number;
  y: number;
  newStroke?: boolean;
}

export interface Annotation {
  id: string;
  x: number;
  y: number;
  pageNumber: number;
  type: AnnotationType;

  // Text properties
  text?: string;
  fontSize?: number;
  fontFamily?: string;
  fontWeight?: string;
  fontStyle?: string;

  // Colors
  color?: string;
  bgColor?: string;
  borderColor?: string;

  // Arrow-specific
  x2?: number;
  y2?: number;
  arrowheadSize?: number;
  strokeWidth?: number;

  // Shape-specific
  shapeType?: ShapeType;
  width?: number;
  height?: number;

  // Highlight-specific
  opacity?: number;
  textRects?: TextRect[];

  // Stamp-specific
  stampType?: StampType;
  stampUsername?: string;
  stampDate?: string;
  stampTime?: string;

  // Freehand-specific
  path?: Point[];

  // Bubble-specific
  bubbleNumber?: number;
  bubbleSize?: number;
  bubbleColor?: string;
  bubbleBorderColor?: string;
  textColor?: string;
  bubbleFontSize?: number;
  hasLeaderLine?: boolean;
  leaderX?: number;
  leaderY?: number;
  leaderColor?: string;
  leaderStrokeWidth?: number;

  // Signature-specific
  signatureMethod?: string;
  signaturePath?: SignaturePoint[];
  signatureImage?: string;
  signatureText?: string;
}

// ---------------------------------------------------------------------------
// Form Templates (match Rust domain::forms)
// ---------------------------------------------------------------------------

export interface FormTemplate {
  id: string;
  tenant_id: string;
  name: string;
  description: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
}

export interface CreateTemplateRequest {
  tenant_id: string;
  name: string;
  description?: string;
  created_by: string;
}

export interface UpdateTemplateRequest {
  name?: string;
  description?: string;
}

export interface ListTemplatesParams {
  tenant_id: string;
  limit?: number;
  offset?: number;
}

// ---------------------------------------------------------------------------
// Form Fields (match Rust domain::forms)
// ---------------------------------------------------------------------------

export type FieldType = 'text' | 'number' | 'date' | 'dropdown' | 'checkbox';

export interface PdfPosition {
  x: number;
  y: number;
  width: number;
  height: number;
  page: number;
}

export interface FormField {
  id: string;
  template_id: string;
  field_key: string;
  field_label: string;
  field_type: FieldType;
  validation_rules: Record<string, unknown>;
  pdf_position: PdfPosition;
  display_order: number;
}

export interface CreateFieldRequest {
  field_key: string;
  field_label: string;
  field_type: FieldType;
  validation_rules?: Record<string, unknown>;
  pdf_position?: PdfPosition;
}

export interface UpdateFieldRequest {
  field_label?: string;
  field_type?: FieldType;
  validation_rules?: Record<string, unknown>;
  pdf_position?: PdfPosition;
}

export interface ReorderFieldsRequest {
  field_ids: string[];
}

// ---------------------------------------------------------------------------
// Form Submissions (match Rust domain::submissions)
// ---------------------------------------------------------------------------

export type SubmissionStatus = 'draft' | 'submitted';

export interface FormSubmission {
  id: string;
  tenant_id: string;
  template_id: string;
  submitted_by: string;
  status: SubmissionStatus;
  field_data: Record<string, unknown>;
  created_at: string;
  submitted_at: string | null;
}

export interface CreateSubmissionRequest {
  tenant_id: string;
  template_id: string;
  submitted_by: string;
  field_data?: Record<string, unknown>;
}

export interface AutosaveRequest {
  field_data: Record<string, unknown>;
}

export interface ListSubmissionsParams {
  tenant_id: string;
  template_id?: string;
  status?: SubmissionStatus;
  limit?: number;
  offset?: number;
}

// ---------------------------------------------------------------------------
// API Error response shape
// ---------------------------------------------------------------------------

export interface ApiError {
  error: string;
  message?: string;
}
