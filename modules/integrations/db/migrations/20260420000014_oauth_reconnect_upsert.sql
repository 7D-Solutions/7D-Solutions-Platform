-- Integrations: OAuth reconnect — partial uniqueness on (provider, realm_id)
--
-- The original UNIQUE(provider, realm_id) was strict: once a realm was connected
-- to any tenant (even a disconnected one), no other tenant could ever connect it.
-- This blocked legitimate cross-tenant reconnect after a disconnect.
--
-- Fix: replace the strict constraint with a partial unique index scoped to
-- connection_status = 'connected'.  Disconnected / needs_reauth rows no longer
-- block other tenants from connecting the same QBO company.
--
-- The UNIQUE(app_id, provider) constraint is unchanged: one row per tenant per
-- provider.  The ON CONFLICT upsert in the application layer handles reconnects
-- by updating the existing row instead of inserting a new one.

ALTER TABLE integrations_oauth_connections
    DROP CONSTRAINT IF EXISTS integrations_oauth_connections_provider_realm_unique;

CREATE UNIQUE INDEX IF NOT EXISTS integrations_oauth_connections_provider_realm_connected
    ON integrations_oauth_connections (provider, realm_id)
    WHERE connection_status = 'connected';
