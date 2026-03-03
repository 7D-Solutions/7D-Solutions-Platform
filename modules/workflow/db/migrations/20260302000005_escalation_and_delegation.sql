-- Escalation rules: per-step timeout configuration.
-- When an instance is at a step for longer than timeout_seconds,
-- an escalation fires targeting escalate_to_step or a notification.
CREATE TABLE workflow_escalation_rules (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    definition_id UUID NOT NULL REFERENCES workflow_definitions(id),
    step_id VARCHAR(100) NOT NULL,
    timeout_seconds INT NOT NULL,
    escalate_to_step VARCHAR(100),
    notify_actor_ids UUID[] NOT NULL DEFAULT '{}',
    notify_template VARCHAR(255),
    max_escalations INT NOT NULL DEFAULT 1,
    is_active BOOLEAN NOT NULL DEFAULT true,
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, definition_id, step_id)
);

CREATE INDEX idx_wf_esc_rules_tenant ON workflow_escalation_rules (tenant_id);
CREATE INDEX idx_wf_esc_rules_def ON workflow_escalation_rules (definition_id);

-- Escalation timers: live timer instances tracking pending escalations.
-- Created when an instance enters a step with an escalation rule.
-- Cancelled when the instance leaves the step.
-- Fired atomically (Guard→Mutation→Outbox) to ensure exactly-once.
CREATE TABLE workflow_escalation_timers (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    instance_id UUID NOT NULL REFERENCES workflow_instances(id),
    rule_id UUID NOT NULL REFERENCES workflow_escalation_rules(id),
    step_id VARCHAR(100) NOT NULL,
    due_at TIMESTAMPTZ NOT NULL,
    fired_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    escalation_count INT NOT NULL DEFAULT 0,
    idempotency_key VARCHAR(512),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Only one active (unfired, uncancelled) timer per instance + rule.
CREATE UNIQUE INDEX idx_wf_esc_timers_active
    ON workflow_escalation_timers (instance_id, rule_id)
    WHERE fired_at IS NULL AND cancelled_at IS NULL;

CREATE INDEX idx_wf_esc_timers_due
    ON workflow_escalation_timers (due_at)
    WHERE fired_at IS NULL AND cancelled_at IS NULL;

CREATE INDEX idx_wf_esc_timers_instance
    ON workflow_escalation_timers (instance_id);

-- Delegation rules: actor A delegates approval authority to actor B.
-- Scoped optionally by definition_id and/or entity_type.
-- Audited: creation and revocation emit events.
CREATE TABLE workflow_delegation_rules (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    delegator_id UUID NOT NULL,
    delegatee_id UUID NOT NULL,
    definition_id UUID REFERENCES workflow_definitions(id),
    entity_type VARCHAR(100),
    reason TEXT,
    valid_from TIMESTAMPTZ NOT NULL DEFAULT now(),
    valid_until TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    revoked_by UUID,
    revoke_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Only one active delegation per (delegator, delegatee, definition, entity_type).
CREATE UNIQUE INDEX idx_wf_delegation_active
    ON workflow_delegation_rules (tenant_id, delegator_id, delegatee_id, definition_id, entity_type)
    WHERE revoked_at IS NULL;

CREATE INDEX idx_wf_delegation_tenant ON workflow_delegation_rules (tenant_id);
CREATE INDEX idx_wf_delegation_delegator ON workflow_delegation_rules (tenant_id, delegator_id)
    WHERE revoked_at IS NULL;
CREATE INDEX idx_wf_delegation_delegatee ON workflow_delegation_rules (tenant_id, delegatee_id)
    WHERE revoked_at IS NULL;
