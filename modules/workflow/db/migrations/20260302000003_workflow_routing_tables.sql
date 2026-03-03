-- Step decisions: records each actor's decision at a workflow step.
-- Used by parallel (N-of-M) routing to count approvals with dedup.
-- UNIQUE constraint prevents the same actor from deciding twice on the same step.
CREATE TABLE workflow_step_decisions (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    instance_id UUID NOT NULL REFERENCES workflow_instances(id),
    step_id VARCHAR(100) NOT NULL,
    actor_id UUID NOT NULL,
    actor_type VARCHAR(50) NOT NULL DEFAULT 'user',
    decision VARCHAR(100) NOT NULL,
    comment TEXT,
    metadata JSONB,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (instance_id, step_id, actor_id)
);

CREATE INDEX idx_wf_step_decisions_instance ON workflow_step_decisions (instance_id, step_id);
CREATE INDEX idx_wf_step_decisions_tenant ON workflow_step_decisions (tenant_id);
