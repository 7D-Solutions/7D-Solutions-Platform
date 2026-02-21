-- Integrations Module: Connector Configurations
--
-- Stores per-tenant connector registrations. A connector is an outbound
-- integration channel (e.g. echo, webhook-push, slack-notify) with a
-- typed config blob validated by the connector's own schema.
--
-- connector_type: discriminator for which connector implementation to use
--   (e.g. 'echo', 'http-push', 'slack').
--
-- config: arbitrary JSONB validated by the connector's config schema.
--   Sensitive fields (secrets, tokens) must NOT be stored in plaintext here —
--   use a secrets manager reference instead.
--
-- Uniqueness: (app_id, connector_type, name) prevents duplicate named
-- connectors of the same type within one tenant.

CREATE TABLE integrations_connector_configs (
    id              UUID NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY,
    app_id          TEXT NOT NULL,
    connector_type  TEXT NOT NULL,      -- e.g. 'echo', 'http-push', 'slack'
    name            TEXT NOT NULL,      -- human-readable label
    config          JSONB NOT NULL DEFAULT '{}'::JSONB,
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT integrations_connector_configs_app_type_name_unique
        UNIQUE (app_id, connector_type, name)
);

-- List active connectors per tenant
CREATE INDEX idx_integrations_connector_configs_app_enabled
    ON integrations_connector_configs(app_id)
    WHERE enabled = TRUE;

-- Filter by type within a tenant
CREATE INDEX idx_integrations_connector_configs_app_type
    ON integrations_connector_configs(app_id, connector_type);
