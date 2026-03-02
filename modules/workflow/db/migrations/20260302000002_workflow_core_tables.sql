-- Workflow definitions (templates)
-- A definition describes the steps, allowed transitions, and metadata
-- for a class of workflows (e.g. "document_approval", "purchase_order").
CREATE TABLE workflow_definitions (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    version INT NOT NULL DEFAULT 1,
    -- steps is a JSONB array of step definitions:
    -- [{ "step_id": "review", "name": "Review", "step_type": "action", "position": 1 }]
    steps JSONB NOT NULL DEFAULT '[]',
    -- initial_step_id: which step instances start on
    initial_step_id VARCHAR(100) NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, name, version)
);

CREATE INDEX idx_wf_defs_tenant ON workflow_definitions (tenant_id);
CREATE INDEX idx_wf_defs_active ON workflow_definitions (tenant_id, is_active)
WHERE is_active = true;

-- Workflow instances
-- A running (or completed) instance of a definition.
CREATE TABLE workflow_instances (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    definition_id UUID NOT NULL REFERENCES workflow_definitions(id),
    -- What entity this workflow is about (e.g. document, purchase order)
    entity_type VARCHAR(100) NOT NULL,
    entity_id VARCHAR(255) NOT NULL,
    current_step_id VARCHAR(100) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    -- CHECK deferred to app layer for flexibility
    context JSONB NOT NULL DEFAULT '{}',
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_wf_instances_tenant ON workflow_instances (tenant_id);
CREATE INDEX idx_wf_instances_def ON workflow_instances (definition_id);
CREATE INDEX idx_wf_instances_entity ON workflow_instances (tenant_id, entity_type, entity_id);
CREATE INDEX idx_wf_instances_status ON workflow_instances (tenant_id, status)
WHERE status = 'active';

-- Workflow transitions (audit trail)
-- Every step change is recorded here for full auditability.
CREATE TABLE workflow_transitions (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    instance_id UUID NOT NULL REFERENCES workflow_instances(id),
    from_step_id VARCHAR(100) NOT NULL,
    to_step_id VARCHAR(100) NOT NULL,
    action VARCHAR(100) NOT NULL,
    actor_id UUID,
    actor_type VARCHAR(50),
    comment TEXT,
    metadata JSONB,
    idempotency_key VARCHAR(512),
    transitioned_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_wf_transitions_instance ON workflow_transitions (instance_id);
CREATE INDEX idx_wf_transitions_tenant ON workflow_transitions (tenant_id);
CREATE UNIQUE INDEX idx_wf_transitions_idempotency
    ON workflow_transitions (tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

-- Idempotency keys for command APIs
CREATE TABLE workflow_idempotency_keys (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(255) NOT NULL DEFAULT 'workflow',
    idempotency_key VARCHAR(512) NOT NULL,
    request_hash VARCHAR(64),
    response_body JSONB NOT NULL,
    status_code INT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL,
    UNIQUE (app_id, idempotency_key)
);
