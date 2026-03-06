-- ECO (Engineering Change Order) tables
-- Manages change control: ECO lifecycle, BOM revision linkage, and doc evidence.
--
-- Status: 'draft' -> 'submitted' -> 'approved' -> 'applied' | 'rejected'
-- Only an approved ECO can be applied to supersede a BOM revision.

CREATE TABLE ecos (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id    TEXT NOT NULL,
    eco_number   TEXT NOT NULL,
    title        TEXT NOT NULL,
    description  TEXT,
    status       TEXT NOT NULL DEFAULT 'draft'
                 CHECK (status IN ('draft', 'submitted', 'approved', 'applied', 'rejected')),
    created_by   TEXT NOT NULL,
    approved_by  TEXT,
    approved_at  TIMESTAMPTZ,
    applied_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT ecos_tenant_number_unique UNIQUE (tenant_id, eco_number)
);

CREATE INDEX idx_ecos_tenant ON ecos(tenant_id);
CREATE INDEX idx_ecos_status ON ecos(status);

-- ECO audit: append-only log of every status transition
CREATE TABLE eco_audit (
    id         BIGSERIAL PRIMARY KEY,
    eco_id     UUID NOT NULL REFERENCES ecos(id),
    tenant_id  TEXT NOT NULL,
    action     TEXT NOT NULL,
    actor      TEXT NOT NULL,
    detail     JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_eco_audit_eco ON eco_audit(eco_id);
CREATE INDEX idx_eco_audit_tenant ON eco_audit(tenant_id);

-- ECO -> BOM revision linkage (before/after)
-- Records which BOM revision was superseded and which new revision was released.
CREATE TABLE eco_bom_revisions (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    eco_id            UUID NOT NULL REFERENCES ecos(id),
    tenant_id         TEXT NOT NULL,
    bom_id            UUID NOT NULL REFERENCES bom_headers(id),
    before_revision_id UUID NOT NULL REFERENCES bom_revisions(id),
    after_revision_id  UUID NOT NULL REFERENCES bom_revisions(id),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT eco_bom_revisions_unique UNIQUE (eco_id, bom_id)
);

CREATE INDEX idx_eco_bom_revisions_eco ON eco_bom_revisions(eco_id);
CREATE INDEX idx_eco_bom_revisions_bom ON eco_bom_revisions(bom_id);

-- ECO -> document revision linkage (evidence)
-- Links released doc revisions to the ECO that drove the change.
CREATE TABLE eco_doc_revisions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    eco_id          UUID NOT NULL REFERENCES ecos(id),
    tenant_id       TEXT NOT NULL,
    doc_id          UUID NOT NULL,
    doc_revision_id UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT eco_doc_revisions_unique UNIQUE (eco_id, doc_revision_id)
);

CREATE INDEX idx_eco_doc_revisions_eco ON eco_doc_revisions(eco_id);
CREATE INDEX idx_eco_doc_revisions_doc ON eco_doc_revisions(doc_id);
