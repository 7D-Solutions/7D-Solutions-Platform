-- Workflow holds: reusable hold/release primitive.
-- Any service can embed holds (quality, engineering, material, customer).
-- Only one active hold per (tenant, entity, hold_type) at a time.
CREATE TABLE workflow_holds (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    entity_type VARCHAR(100) NOT NULL,
    entity_id VARCHAR(255) NOT NULL,
    hold_type VARCHAR(100) NOT NULL,
    reason TEXT,
    applied_by UUID,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_by UUID,
    released_at TIMESTAMPTZ,
    release_reason TEXT,
    idempotency_key VARCHAR(512),
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Only one active (unreleased) hold per entity + hold_type per tenant.
CREATE UNIQUE INDEX idx_wf_holds_active_unique
    ON workflow_holds (tenant_id, entity_type, entity_id, hold_type)
    WHERE released_at IS NULL;

CREATE INDEX idx_wf_holds_tenant ON workflow_holds (tenant_id);
CREATE INDEX idx_wf_holds_entity ON workflow_holds (tenant_id, entity_type, entity_id);
CREATE INDEX idx_wf_holds_active ON workflow_holds (tenant_id, entity_type, entity_id)
    WHERE released_at IS NULL;
