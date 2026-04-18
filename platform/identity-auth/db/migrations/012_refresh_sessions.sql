-- Refresh sessions: sliding-expiry session tracking for long-lived refresh cookies.
--
-- Distinct from refresh_tokens (legacy body-based refresh) — refresh_sessions is
-- the backing store for the HttpOnly-cookie refresh flow with:
--   - sliding idle timeout (last_used_at + REFRESH_IDLE_MINUTES)
--   - hard absolute maximum lifetime (issued_at + REFRESH_ABSOLUTE_MAX_DAYS)
--   - device_info for a future per-device session UI
--   - revocation_reason for audit / replay-detection provenance
--
-- See docs/architecture/IDENTITY-AUTH-REFRESH-TOKENS-SPEC.md.

CREATE TABLE refresh_sessions (
    session_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL,
    token_hash TEXT NOT NULL,
    device_info JSONB NOT NULL DEFAULT '{}'::jsonb,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    absolute_expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    revocation_reason TEXT,
    CONSTRAINT refresh_sessions_token_hash_unique UNIQUE (token_hash)
);

CREATE INDEX idx_refresh_sessions_user
    ON refresh_sessions (user_id, tenant_id);

CREATE INDEX idx_refresh_sessions_user_active
    ON refresh_sessions (user_id, tenant_id)
    WHERE revoked_at IS NULL;

CREATE INDEX idx_refresh_sessions_token_hash
    ON refresh_sessions (token_hash);
