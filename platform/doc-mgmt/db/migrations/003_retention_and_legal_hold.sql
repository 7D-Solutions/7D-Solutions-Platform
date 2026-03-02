-- DOC2: Retention management + legal hold.
--
-- 1. retention_policies: per-tenant, per-doc_type retention rules.
-- 2. legal_holds: document-level holds with durable audit trail.
-- 3. DB trigger: block disposal while any active hold exists.
-- 4. Add 'disposed' to allowed document statuses.

-- ── 1. Retention policies ────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS retention_policies (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    doc_type        VARCHAR(64) NOT NULL,
    retention_days  INTEGER NOT NULL CHECK (retention_days > 0),
    created_by      UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (tenant_id, doc_type)
);

CREATE INDEX IF NOT EXISTS idx_retention_policies_tenant
    ON retention_policies (tenant_id);

-- ── 2. Legal holds ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS legal_holds (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id     UUID NOT NULL REFERENCES documents(id),
    tenant_id       UUID NOT NULL,
    reason          TEXT NOT NULL,
    held_by         UUID NOT NULL,
    held_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_by     UUID,
    released_at     TIMESTAMPTZ,

    -- Only one active hold per document+reason (released_at IS NULL = active)
    CONSTRAINT uq_active_hold_per_reason UNIQUE NULLS NOT DISTINCT (document_id, reason, released_at)
);

CREATE INDEX IF NOT EXISTS idx_legal_holds_document
    ON legal_holds (document_id) WHERE released_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_legal_holds_tenant
    ON legal_holds (tenant_id);

-- ── 3. DB trigger: block disposal while active holds exist ───────────
CREATE OR REPLACE FUNCTION prevent_dispose_with_active_hold()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.status = 'disposed' AND (OLD.status IS DISTINCT FROM 'disposed') THEN
        IF EXISTS (
            SELECT 1 FROM legal_holds
            WHERE document_id = NEW.id
              AND tenant_id = NEW.tenant_id
              AND released_at IS NULL
        ) THEN
            RAISE EXCEPTION 'Cannot dispose document (id=%) — active legal hold exists', NEW.id
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_block_dispose_with_hold ON documents;
CREATE TRIGGER trg_block_dispose_with_hold
    BEFORE UPDATE ON documents
    FOR EACH ROW
    EXECUTE FUNCTION prevent_dispose_with_active_hold();

-- Also prevent DELETE of documents with active holds.
CREATE OR REPLACE FUNCTION prevent_delete_with_active_hold()
RETURNS TRIGGER AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM legal_holds
        WHERE document_id = OLD.id
          AND tenant_id = OLD.tenant_id
          AND released_at IS NULL
    ) THEN
        RAISE EXCEPTION 'Cannot delete document (id=%) — active legal hold exists', OLD.id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_block_delete_with_hold ON documents;
CREATE TRIGGER trg_block_delete_with_hold
    BEFORE DELETE ON documents
    FOR EACH ROW
    EXECUTE FUNCTION prevent_delete_with_active_hold();
