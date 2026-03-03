-- DOC3: Template engine — document formatting/render pipeline.
--
-- 1. doc_templates: reusable template definitions with versioning.
-- 2. render_artifacts: deterministic, auditable rendered output.
-- 3. Idempotent rendering via (tenant_id, idempotency_key) uniqueness.

-- ── 1. Templates ──────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS doc_templates (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    name            VARCHAR(256) NOT NULL,
    doc_type        VARCHAR(64) NOT NULL,
    body_template   JSONB NOT NULL DEFAULT '{}',
    version         INTEGER NOT NULL DEFAULT 1,
    created_by      UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (tenant_id, name, version)
);

CREATE INDEX IF NOT EXISTS idx_doc_templates_tenant
    ON doc_templates (tenant_id);

CREATE INDEX IF NOT EXISTS idx_doc_templates_doc_type
    ON doc_templates (tenant_id, doc_type);

-- ── 2. Render artifacts ───────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS render_artifacts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    template_id     UUID NOT NULL REFERENCES doc_templates(id),
    idempotency_key VARCHAR(512),
    input_hash      VARCHAR(64) NOT NULL,
    output_hash     VARCHAR(64) NOT NULL,
    output          JSONB NOT NULL,
    rendered_by     UUID NOT NULL,
    rendered_at     TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_render_artifacts_tenant
    ON render_artifacts (tenant_id);

CREATE INDEX IF NOT EXISTS idx_render_artifacts_template
    ON render_artifacts (template_id);
