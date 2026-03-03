-- Item Classifications & Commodity Codes
--
-- Enables items to be tagged with classification categories and
-- standard commodity codes (UNSPSC, NAICS, HS, etc.) for reporting,
-- compliance, and downstream routing.
--
-- Design decisions:
--   - Classifications are assigned at the item level (not revision level)
--     because classification is a governance/taxonomy concern that persists
--     across revisions. The bead says "revision-aware where appropriate" —
--     we link to revision_id optionally so the assignment can record which
--     revision was current when the classification was set.
--   - Each (tenant, item, classification_system, code) tuple is unique
--     to prevent duplicate assignments.
--   - Idempotent via idempotency_key (unique per tenant).

CREATE TABLE item_classifications (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             TEXT NOT NULL,
    item_id               UUID NOT NULL REFERENCES items(id),
    revision_id           UUID REFERENCES item_revisions(id),

    -- Classification system (e.g. "internal", "department", "product_line")
    classification_system TEXT NOT NULL,
    -- Classification code within that system
    classification_code   TEXT NOT NULL,
    -- Human-readable label (optional)
    classification_label  TEXT,

    -- Commodity code system (e.g. "UNSPSC", "NAICS", "HS", "ECCN")
    commodity_system      TEXT,
    -- The actual commodity code value
    commodity_code        TEXT,

    -- Audit
    assigned_by           TEXT NOT NULL DEFAULT 'system',
    idempotency_key       TEXT,
    created_at            TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- No duplicate classification for the same system+code on one item
    CONSTRAINT item_classifications_unique_assignment
        UNIQUE (tenant_id, item_id, classification_system, classification_code),

    -- Idempotency key unique per tenant
    CONSTRAINT item_classifications_tenant_idemp_unique
        UNIQUE (tenant_id, idempotency_key)
);

-- Query pattern: list all classifications for an item
CREATE INDEX idx_item_classifications_item
    ON item_classifications(tenant_id, item_id);

-- Query pattern: find items by classification
CREATE INDEX idx_item_classifications_system_code
    ON item_classifications(tenant_id, classification_system, classification_code);

-- Query pattern: find items by commodity code
CREATE INDEX idx_item_classifications_commodity
    ON item_classifications(tenant_id, commodity_system, commodity_code)
    WHERE commodity_system IS NOT NULL;
