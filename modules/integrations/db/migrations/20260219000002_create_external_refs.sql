-- Integrations Module: External References Registry
--
-- Maps internal platform records (invoices, customers, orders, etc.) to
-- identifiers in external systems (Stripe, QuickBooks, Salesforce, etc.).
--
-- This is the global cross-entity mapping table. Unlike party_external_refs
-- (which is scoped to parties), this covers any entity type in the platform.
--
-- Uniqueness: (app_id, system, external_id) prevents the same external
-- identifier from being claimed by more than one internal record within a
-- given app + system combination.

CREATE TABLE integrations_external_refs (
    id           BIGSERIAL PRIMARY KEY,
    app_id       TEXT    NOT NULL,
    entity_type  TEXT    NOT NULL,  -- e.g. 'invoice', 'customer', 'order', 'party'
    entity_id    TEXT    NOT NULL,  -- UUID or opaque ID of the internal record
    system       TEXT    NOT NULL,  -- e.g. 'stripe', 'quickbooks', 'salesforce'
    external_id  TEXT    NOT NULL,  -- identifier in the external system
    label        TEXT,              -- optional human-readable label
    metadata     JSONB,
    created_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- One external ID per (app, system) — prevents duplicate mappings
    CONSTRAINT integrations_external_refs_app_system_id_unique
        UNIQUE (app_id, system, external_id)
);

-- Look up all external refs for a given internal record
CREATE INDEX idx_integrations_ext_refs_entity
    ON integrations_external_refs(app_id, entity_type, entity_id);

-- Look up by external system (e.g. "find all Stripe refs for this app")
CREATE INDEX idx_integrations_ext_refs_system
    ON integrations_external_refs(app_id, system);

-- Partial index to find recently updated refs
CREATE INDEX idx_integrations_ext_refs_updated
    ON integrations_external_refs(updated_at);
