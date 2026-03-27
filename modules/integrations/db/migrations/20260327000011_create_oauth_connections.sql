-- Integrations Module: OAuth Connections
--
-- Per-tenant OAuth connection storage for external providers (e.g. QuickBooks).
-- Tokens are encrypted at rest using pgcrypto pgp_sym_encrypt.
--
-- Key invariants:
--   1. One QBO company (realm_id) can only connect to one tenant globally.
--   2. One tenant can only have one active connection per provider.
--   3. Tokens are never stored as plaintext — always BYTEA ciphertext.
--   4. Concurrent refresh is prevented via SELECT ... FOR UPDATE SKIP LOCKED.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE integrations_oauth_connections (
    id                        UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id                    TEXT        NOT NULL,
    provider                  TEXT        NOT NULL,
    realm_id                  TEXT        NOT NULL,
    access_token              BYTEA       NOT NULL,
    refresh_token             BYTEA       NOT NULL,
    access_token_expires_at   TIMESTAMPTZ NOT NULL,
    refresh_token_expires_at  TIMESTAMPTZ NOT NULL,
    scopes_granted            TEXT        NOT NULL,
    connection_status         TEXT        NOT NULL DEFAULT 'connected',
    last_successful_refresh   TIMESTAMPTZ,
    cdc_watermark             TIMESTAMPTZ,
    full_resync_required      BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at                TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Status must be a known value
    CONSTRAINT integrations_oauth_connections_status_check
        CHECK (connection_status IN ('connected', 'disconnected', 'needs_reauth')),

    -- Globally, one QBO company can only connect to one tenant
    CONSTRAINT integrations_oauth_connections_provider_realm_unique
        UNIQUE (provider, realm_id),

    -- One active connection per tenant per provider
    CONSTRAINT integrations_oauth_connections_app_provider_unique
        UNIQUE (app_id, provider)
);

-- Refresh worker query: find connections needing token refresh
CREATE INDEX idx_integrations_oauth_connections_refresh
    ON integrations_oauth_connections (connection_status, access_token_expires_at)
    WHERE connection_status = 'connected';
