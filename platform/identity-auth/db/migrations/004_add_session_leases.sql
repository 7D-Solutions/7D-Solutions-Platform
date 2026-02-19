-- Session leases: authoritative DB-backed concurrent seat enforcement.
-- Each active refresh token has a corresponding lease row.
-- Active leases = revoked_at IS NULL AND last_seen_at >= NOW() - INTERVAL '30 minutes'.
CREATE TABLE session_leases (
    lease_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   UUID NOT NULL,
    user_id     UUID NOT NULL,
    session_id  UUID NOT NULL REFERENCES refresh_tokens(id) ON DELETE CASCADE,
    issued_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at  TIMESTAMPTZ
);

-- Fast active-lease count per tenant
CREATE INDEX idx_session_leases_tenant_active
    ON session_leases(tenant_id, last_seen_at)
    WHERE revoked_at IS NULL;

-- Fast lookup by refresh token id (for rotate + revoke)
CREATE INDEX idx_session_leases_session
    ON session_leases(session_id);
