-- Item Revisions: revisioned item definitions with effective dating
--
-- Each item can have multiple revisions. A revision captures a snapshot of
-- the item's definition (name, UoM, GL accounts) at a point in time.
--
-- Effective dating:
--   effective_from  = NULL  → revision is a draft (not yet activated)
--   effective_from != NULL, effective_to = NULL → currently effective (open-ended)
--   effective_from != NULL, effective_to != NULL → historically effective (closed window)
--
-- Invariant: for a given (tenant_id, item_id), no two activated revisions
-- may have overlapping [effective_from, effective_to) windows. This is
-- enforced via an exclusion constraint using tstzrange.

CREATE EXTENSION IF NOT EXISTS btree_gist;

CREATE TABLE item_revisions (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id              TEXT NOT NULL,
    item_id                UUID NOT NULL REFERENCES items(id),
    revision_number        INT NOT NULL,

    -- Snapshot of item definition at this revision
    name                   TEXT NOT NULL,
    description            TEXT,
    uom                    TEXT NOT NULL,
    inventory_account_ref  TEXT NOT NULL,
    cogs_account_ref       TEXT NOT NULL,
    variance_account_ref   TEXT NOT NULL,

    -- Effective dating
    effective_from         TIMESTAMP WITH TIME ZONE,
    effective_to           TIMESTAMP WITH TIME ZONE,

    -- Metadata
    change_reason          TEXT NOT NULL,
    idempotency_key        TEXT,
    created_at             TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    activated_at           TIMESTAMP WITH TIME ZONE,

    -- Unique revision number per item per tenant
    CONSTRAINT item_revisions_tenant_item_rev_unique
        UNIQUE (tenant_id, item_id, revision_number),

    -- Idempotency key unique per tenant (NULL keys are exempt)
    CONSTRAINT item_revisions_tenant_idemp_unique
        UNIQUE (tenant_id, idempotency_key),

    -- effective_to must be after effective_from when both are set
    CONSTRAINT item_revisions_effective_order
        CHECK (effective_to IS NULL OR effective_from < effective_to)
);

-- Exclusion constraint: no overlapping effective windows for the same item+tenant.
-- Uses half-open interval [effective_from, effective_to) where NULL effective_to
-- maps to 'infinity' (open-ended). Only applies to activated revisions.
ALTER TABLE item_revisions
    ADD CONSTRAINT item_revisions_no_overlap
    EXCLUDE USING gist (
        tenant_id WITH =,
        item_id WITH =,
        tstzrange(effective_from, COALESCE(effective_to, 'infinity'::timestamptz), '[)') WITH &&
    ) WHERE (effective_from IS NOT NULL);

-- Query pattern: "current revision at time T"
CREATE INDEX idx_item_revisions_effective_lookup
    ON item_revisions(tenant_id, item_id, effective_from, effective_to);

-- Query pattern: list all revisions for an item
CREATE INDEX idx_item_revisions_item
    ON item_revisions(tenant_id, item_id, revision_number);
