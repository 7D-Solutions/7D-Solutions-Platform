-- BOM module: header + revisions + lines + outbox
--
-- BOM header: one per part per tenant.
-- BOM revision: versioned snapshot of the BOM structure with date-based effectivity.
-- BOM lines: components belonging to a specific revision.
--
-- Effectivity: non-overlapping date ranges per BOM header enforced by an exclusion
-- constraint using tstzrange. Only revisions with status = 'effective' participate.

-- BOM header: identifies which part has a bill of materials
CREATE TABLE bom_headers (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   TEXT NOT NULL,
    part_id     UUID NOT NULL,
    description TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT bom_headers_tenant_part_unique UNIQUE (tenant_id, part_id)
);

CREATE INDEX idx_bom_headers_tenant ON bom_headers(tenant_id);

-- BOM revision: a versioned snapshot of BOM structure
-- status: 'draft' | 'effective' | 'superseded'
-- effective_from / effective_to define when this revision is active.
-- Only one 'effective' revision per BOM header for any given date.
CREATE TABLE bom_revisions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bom_id          UUID NOT NULL REFERENCES bom_headers(id),
    tenant_id       TEXT NOT NULL,
    revision_label  TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'draft'
                    CHECK (status IN ('draft', 'effective', 'superseded')),
    effective_from  TIMESTAMPTZ,
    effective_to    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT bom_revisions_label_unique UNIQUE (bom_id, revision_label),
    CONSTRAINT bom_revisions_effective_range_valid
        CHECK (effective_to IS NULL OR effective_from IS NULL OR effective_to > effective_from)
);

CREATE INDEX idx_bom_revisions_bom ON bom_revisions(bom_id);
CREATE INDEX idx_bom_revisions_tenant ON bom_revisions(tenant_id);
CREATE INDEX idx_bom_revisions_status ON bom_revisions(status);

-- Exclusion constraint: no two 'effective' revisions for the same BOM can overlap.
-- We use btree_gist to combine equality (bom_id) with range overlap (tstzrange).
CREATE EXTENSION IF NOT EXISTS btree_gist;

ALTER TABLE bom_revisions ADD CONSTRAINT bom_revisions_effectivity_excl
    EXCLUDE USING gist (
        bom_id WITH =,
        tstzrange(effective_from, effective_to, '[)') WITH &&
    )
    WHERE (status = 'effective');

-- BOM line: a component within a revision
CREATE TABLE bom_lines (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    revision_id         UUID NOT NULL REFERENCES bom_revisions(id) ON DELETE CASCADE,
    tenant_id           TEXT NOT NULL,
    component_item_id   UUID NOT NULL,
    quantity            NUMERIC(18, 6) NOT NULL CHECK (quantity > 0),
    uom                 TEXT,
    scrap_factor        NUMERIC(5, 4) DEFAULT 0 CHECK (scrap_factor >= 0 AND scrap_factor < 1),
    find_number         INT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT bom_lines_revision_component_unique UNIQUE (revision_id, component_item_id)
);

CREATE INDEX idx_bom_lines_revision ON bom_lines(revision_id);
CREATE INDEX idx_bom_lines_component ON bom_lines(component_item_id);
CREATE INDEX idx_bom_lines_tenant ON bom_lines(tenant_id);

-- Outbox for BOM events
CREATE TABLE bom_outbox (
    id              BIGSERIAL PRIMARY KEY,
    event_id        UUID NOT NULL UNIQUE,
    event_type      TEXT NOT NULL,
    aggregate_type  TEXT NOT NULL,
    aggregate_id    TEXT NOT NULL,
    tenant_id       TEXT NOT NULL,
    payload         JSONB NOT NULL,
    correlation_id  TEXT,
    causation_id    TEXT,
    schema_version  TEXT NOT NULL DEFAULT '1.0.0',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at    TIMESTAMPTZ
);

CREATE INDEX idx_bom_outbox_unpublished ON bom_outbox(created_at) WHERE published_at IS NULL;
CREATE INDEX idx_bom_outbox_tenant ON bom_outbox(tenant_id);

-- Processed events for idempotent consumers
CREATE TABLE bom_processed_events (
    id           BIGSERIAL PRIMARY KEY,
    event_id     UUID NOT NULL UNIQUE,
    event_type   TEXT NOT NULL,
    processor    TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_bom_processed_event_id ON bom_processed_events(event_id);
