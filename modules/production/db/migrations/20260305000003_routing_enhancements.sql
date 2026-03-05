-- Routing enhancements: add revision, status, effectivity to routing_templates;
-- add is_required flag to routing_steps.

ALTER TABLE routing_templates
    ADD COLUMN IF NOT EXISTS revision TEXT NOT NULL DEFAULT '1',
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'draft',
    ADD COLUMN IF NOT EXISTS effective_from_date DATE;

ALTER TABLE routing_steps
    ADD COLUMN IF NOT EXISTS is_required BOOLEAN NOT NULL DEFAULT TRUE;

-- Index for querying routings by part (item_id) + effective date
CREATE INDEX IF NOT EXISTS idx_routing_templates_item_effective
    ON routing_templates (tenant_id, item_id, effective_from_date);

-- Unique constraint: one revision per item per tenant
CREATE UNIQUE INDEX IF NOT EXISTS idx_routing_templates_item_revision
    ON routing_templates (tenant_id, item_id, revision);
