-- Immutable user lifecycle audit timeline + outbox for compliance-grade traceability.

CREATE TABLE user_lifecycle_audit_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL,
    event_type VARCHAR(64) NOT NULL,
    actor_user_id UUID,
    role_id UUID,
    review_id UUID,
    decision VARCHAR(64),
    idempotency_key VARCHAR(255) NOT NULL,
    event_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_user_lifecycle_event_type CHECK (
        event_type IN (
            'user_created',
            'role_assigned',
            'role_revoked',
            'access_review_recorded'
        )
    ),
    CONSTRAINT chk_user_lifecycle_idempotency_non_empty CHECK (length(trim(idempotency_key)) > 0),
    CONSTRAINT uq_user_lifecycle_idempotency UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_user_lifecycle_timeline
    ON user_lifecycle_audit_events(tenant_id, user_id, occurred_at, created_at, id);

CREATE INDEX idx_user_lifecycle_by_type
    ON user_lifecycle_audit_events(tenant_id, event_type, occurred_at);

CREATE TABLE user_lifecycle_events_outbox (
    id BIGSERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    tenant_id UUID NOT NULL,
    aggregate_id UUID NOT NULL,
    event_type VARCHAR(128) NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at TIMESTAMPTZ
);

CREATE INDEX idx_user_lifecycle_outbox_unpublished
    ON user_lifecycle_events_outbox(created_at)
    WHERE published_at IS NULL;

CREATE OR REPLACE FUNCTION prevent_user_lifecycle_audit_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'user_lifecycle_audit_events is immutable';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_user_lifecycle_no_update
BEFORE UPDATE ON user_lifecycle_audit_events
FOR EACH ROW EXECUTE FUNCTION prevent_user_lifecycle_audit_mutation();

CREATE TRIGGER trg_user_lifecycle_no_delete
BEFORE DELETE ON user_lifecycle_audit_events
FOR EACH ROW EXECUTE FUNCTION prevent_user_lifecycle_audit_mutation();
