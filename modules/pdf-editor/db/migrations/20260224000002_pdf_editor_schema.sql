-- PDF Editor Schema v1
-- Tables: pdf_documents, annotation_sets, form_templates,
-- form_fields, form_submissions, generated_documents.
-- Every row is tenant-scoped. annotation_sets has version
-- column for optimistic locking.

-- ============================================================
-- PDF DOCUMENTS
-- Metadata for uploaded PDFs. File bytes live in S3.
-- storage_key = S3 object key (tenant/{tid}/pdf/{id}/original.pdf).
-- ============================================================

CREATE TABLE pdf_documents (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    name            TEXT NOT NULL,
    storage_key     TEXT NOT NULL,
    content_type    TEXT NOT NULL DEFAULT 'application/pdf',
    size_bytes      BIGINT NOT NULL,
    uploaded_by     TEXT NOT NULL,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_pdf_documents_tenant ON pdf_documents(tenant_id);
CREATE INDEX idx_pdf_documents_tenant_created ON pdf_documents(tenant_id, created_at DESC);

-- ============================================================
-- ANNOTATION SETS
-- One set per document. annotations is a JSONB array of
-- annotation objects (type, position, content, style).
-- version column enables optimistic locking via If-Match / ETag.
-- ============================================================

CREATE TABLE annotation_sets (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    pdf_document_id   UUID NOT NULL REFERENCES pdf_documents(id),
    created_by        TEXT NOT NULL,
    annotations       JSONB NOT NULL DEFAULT '[]'::jsonb,
    version           INTEGER NOT NULL DEFAULT 1,
    created_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT annotation_sets_document_unique UNIQUE (pdf_document_id)
);

CREATE INDEX idx_annotation_sets_tenant ON annotation_sets(tenant_id);
CREATE INDEX idx_annotation_sets_document ON annotation_sets(pdf_document_id);

-- ============================================================
-- FORM TEMPLATES
-- Reusable form definitions tied to a PDF document.
-- Fields are stored in a separate table for ordering.
-- ============================================================

CREATE TABLE form_templates (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    name              TEXT NOT NULL,
    description       TEXT,
    pdf_document_id   UUID NOT NULL REFERENCES pdf_documents(id),
    created_by        TEXT NOT NULL,
    created_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_form_templates_tenant ON form_templates(tenant_id);
CREATE INDEX idx_form_templates_tenant_name ON form_templates(tenant_id, name);
CREATE INDEX idx_form_templates_document ON form_templates(pdf_document_id);

-- ============================================================
-- FORM FIELDS
-- Individual fields within a form template.
-- pdf_position stores {x, y, width, height, page}.
-- validation_rules stores per-field rules (required, min, max, pattern, options).
-- display_order controls rendering sequence.
-- ============================================================

CREATE TABLE form_fields (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    template_id       UUID NOT NULL REFERENCES form_templates(id) ON DELETE CASCADE,
    field_key         TEXT NOT NULL,
    field_label       TEXT NOT NULL,
    field_type        TEXT NOT NULL
                      CHECK (field_type IN ('text', 'number', 'date', 'dropdown', 'checkbox')),
    validation_rules  JSONB NOT NULL DEFAULT '{}'::jsonb,
    pdf_position      JSONB NOT NULL DEFAULT '{}'::jsonb,
    display_order     INTEGER NOT NULL DEFAULT 0,

    CONSTRAINT form_fields_template_key_unique UNIQUE (template_id, field_key)
);

CREATE INDEX idx_form_fields_template ON form_fields(template_id);
CREATE INDEX idx_form_fields_template_order ON form_fields(template_id, display_order);

-- ============================================================
-- FORM SUBMISSIONS
-- Data filled against a template. draft → submitted.
-- field_data stores {field_key: value} pairs.
-- submitted_at is set when status transitions to 'submitted'.
-- ============================================================

CREATE TABLE form_submissions (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    template_id       UUID NOT NULL REFERENCES form_templates(id),
    submitted_by      TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'draft'
                      CHECK (status IN ('draft', 'submitted')),
    field_data        JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    submitted_at      TIMESTAMP WITH TIME ZONE
);

CREATE INDEX idx_form_submissions_tenant ON form_submissions(tenant_id);
CREATE INDEX idx_form_submissions_tenant_status ON form_submissions(tenant_id, status);
CREATE INDEX idx_form_submissions_template ON form_submissions(template_id);

-- ============================================================
-- GENERATED DOCUMENTS
-- Output PDFs created from annotations or form submissions.
-- source_type indicates what produced the PDF.
-- source_id points to either an annotation_sets.id or form_submissions.id.
-- storage_key = S3 object key (tenant/{tid}/generated/{id}.pdf).
-- ============================================================

CREATE TABLE generated_documents (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    source_type       TEXT NOT NULL
                      CHECK (source_type IN ('submission', 'annotation')),
    source_id         UUID NOT NULL,
    storage_key       TEXT NOT NULL,
    created_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_generated_documents_tenant ON generated_documents(tenant_id);
CREATE INDEX idx_generated_documents_source ON generated_documents(source_type, source_id);
CREATE INDEX idx_generated_documents_tenant_created ON generated_documents(tenant_id, created_at DESC);
