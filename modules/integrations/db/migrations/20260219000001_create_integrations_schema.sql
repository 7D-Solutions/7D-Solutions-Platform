-- Integrations module initial schema placeholder.
-- Full schema (external_refs, webhook_endpoints, webhook_ingest, outbox/idempotency)
-- is created in bd-b992.

-- Idempotency key tracking table (minimal bootstrap)
CREATE TABLE IF NOT EXISTS integrations_schema_version (
    id          BIGSERIAL PRIMARY KEY,
    version     TEXT NOT NULL,
    applied_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO integrations_schema_version (version) VALUES ('0.1.0-scaffold');
