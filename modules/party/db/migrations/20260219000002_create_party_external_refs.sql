-- Party Module: External References
--
-- Maps party records to identifiers in external systems
-- (e.g. CRM, ERP, Stripe, QuickBooks).
--
-- The uniqueness constraint is (app_id, system, external_id) — the same
-- external identifier cannot be claimed by more than one party within a
-- given app + system combination.

CREATE TABLE party_external_refs (
    id          BIGSERIAL PRIMARY KEY,
    party_id    UUID NOT NULL REFERENCES party_parties(id) ON DELETE CASCADE,
    app_id      TEXT NOT NULL,
    system      TEXT NOT NULL,     -- e.g. 'stripe', 'quickbooks', 'salesforce'
    external_id TEXT NOT NULL,     -- identifier in the external system
    label       TEXT,              -- optional human-readable label
    metadata    JSONB,
    created_at  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- One external ID per (app, system) — prevents duplicate mappings
    CONSTRAINT party_external_refs_app_system_id_unique
        UNIQUE (app_id, system, external_id)
);

CREATE INDEX idx_party_external_refs_party_id  ON party_external_refs(party_id);
CREATE INDEX idx_party_external_refs_app_id    ON party_external_refs(app_id);
CREATE INDEX idx_party_external_refs_system    ON party_external_refs(system);
CREATE INDEX idx_party_external_refs_app_party ON party_external_refs(app_id, party_id);
