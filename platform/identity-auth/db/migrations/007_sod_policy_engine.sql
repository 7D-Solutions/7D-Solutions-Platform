-- Separation of Duties policy engine: tenant policy model, idempotent mutation/decision logs,
-- and outbox for envelope-complete event emission.

CREATE TABLE sod_policies (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    action_key VARCHAR(128) NOT NULL,
    primary_role_id UUID NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    conflicting_role_id UUID NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    allow_override BOOLEAN NOT NULL DEFAULT false,
    override_requires_approval BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_sod_distinct_roles CHECK (primary_role_id <> conflicting_role_id)
);

CREATE UNIQUE INDEX uq_sod_policy_pair
    ON sod_policies (
        tenant_id,
        action_key,
        LEAST(primary_role_id, conflicting_role_id),
        GREATEST(primary_role_id, conflicting_role_id)
    );

CREATE INDEX idx_sod_policies_tenant_action ON sod_policies(tenant_id, action_key);

CREATE TABLE sod_policy_mutations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    idempotency_key VARCHAR(255) NOT NULL,
    policy_id UUID,
    actor_user_id UUID,
    mutation_type VARCHAR(32) NOT NULL,
    mutation_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_sod_mutation_type CHECK (mutation_type IN ('policy_upsert', 'policy_delete')),
    CONSTRAINT chk_sod_mutation_idem_non_empty CHECK (length(trim(idempotency_key)) > 0),
    CONSTRAINT uq_sod_policy_mutation_idem UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_sod_policy_mutations_tenant_time
    ON sod_policy_mutations(tenant_id, occurred_at);

CREATE TABLE sod_decision_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    idempotency_key VARCHAR(255) NOT NULL,
    action_key VARCHAR(128) NOT NULL,
    actor_user_id UUID NOT NULL,
    subject_user_id UUID,
    decision VARCHAR(32) NOT NULL,
    reason TEXT NOT NULL,
    matched_policy_ids UUID[] NOT NULL DEFAULT '{}',
    override_granted_by UUID,
    override_ticket TEXT,
    decision_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_sod_decision CHECK (decision IN ('allow', 'deny', 'allow_with_override')),
    CONSTRAINT chk_sod_decision_idem_non_empty CHECK (length(trim(idempotency_key)) > 0),
    CONSTRAINT uq_sod_decision_idem UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_sod_decision_tenant_actor
    ON sod_decision_logs(tenant_id, actor_user_id, occurred_at);

CREATE TABLE sod_events_outbox (
    id BIGSERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    tenant_id UUID NOT NULL,
    aggregate_id UUID NOT NULL,
    event_type VARCHAR(128) NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at TIMESTAMPTZ
);

CREATE INDEX idx_sod_events_outbox_unpublished
    ON sod_events_outbox(created_at)
    WHERE published_at IS NULL;
