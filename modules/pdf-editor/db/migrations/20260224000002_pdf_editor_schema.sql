-- PDF Editor Schema v1 (Rev 2.0 — stateless processing engine)
--
-- Tables: form_templates, form_fields, form_submissions.
-- Every row is tenant-scoped. Editor stores NO PDF files.

-- ============================================================
-- FORM TEMPLATES
-- Reusable form definitions (field layouts, validation rules).
-- NO reference to any PDF document — the PDF is provided at
-- generation time by the caller.
-- ============================================================

CREATE TABLE form_templates (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    name              TEXT NOT NULL,
    description       TEXT,
    created_by        TEXT NOT NULL,
    created_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_form_templates_tenant ON form_templates(tenant_id);
CREATE INDEX idx_form_templates_tenant_name ON form_templates(tenant_id, name);

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
