-- Acceptance Authority Register
--
-- Models "who is permitted to accept/approve inspection results for scope S."
-- Grants are time-bounded, revocable, and auditable.
-- Never delete rows — revocations set is_revoked + revoked_at for audit trail.

CREATE TABLE wc_acceptance_authorities (
    id                UUID PRIMARY KEY,
    tenant_id         TEXT NOT NULL,
    operator_id       UUID NOT NULL,
    capability_scope  TEXT NOT NULL,
    constraints       JSONB,
    effective_from    TIMESTAMP WITH TIME ZONE NOT NULL,
    effective_until   TIMESTAMP WITH TIME ZONE,
    granted_by        TEXT,
    is_revoked        BOOLEAN NOT NULL DEFAULT false,
    revoked_at        TIMESTAMP WITH TIME ZONE,
    revocation_reason TEXT,
    idempotency_key   TEXT NOT NULL,
    created_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT wc_aa_tenant_idem_unique UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_wc_aa_tenant ON wc_acceptance_authorities(tenant_id);
CREATE INDEX idx_wc_aa_tenant_operator ON wc_acceptance_authorities(tenant_id, operator_id);
CREATE INDEX idx_wc_aa_scope ON wc_acceptance_authorities(tenant_id, capability_scope);

-- Authorization lookup: tenant + operator + scope + not revoked + time range
CREATE INDEX idx_wc_aa_auth_lookup ON wc_acceptance_authorities(tenant_id, operator_id, capability_scope)
    WHERE is_revoked = false;
