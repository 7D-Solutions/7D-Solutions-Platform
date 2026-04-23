-- Per-tenant QBO webhook verifier token storage.
-- realm_id = '' is the app-wide fallback row; non-empty realm_id is a per-realm override.
-- Lookup order: exact (app_id, realm_id) first, then (app_id, '') as fallback.
CREATE TABLE IF NOT EXISTS integrations_qbo_webhook_secrets (
    app_id         TEXT        NOT NULL,
    realm_id       TEXT        NOT NULL DEFAULT '',
    token_enc      BYTEA       NOT NULL,
    configured_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app_id, realm_id)
);
