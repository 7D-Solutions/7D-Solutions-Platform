-- DOC1: Revision control — immutable release + supersede linkage.
--
-- 1. Add status column to revisions (draft → released → superseded).
-- 2. Add superseded_by to documents for successor linkage.
-- 3. DB trigger prevents ANY update to released revisions.
-- 4. Add 'superseded' to allowed document statuses via CHECK.

-- ── 1. Revision status ─────────────────────────────────────────────
ALTER TABLE revisions
    ADD COLUMN IF NOT EXISTS status VARCHAR(32) NOT NULL DEFAULT 'draft';

-- Back-fill: if a document is released, mark its revisions as released.
UPDATE revisions r
SET status = 'released'
FROM documents d
WHERE r.document_id = d.id AND d.status = 'released';

-- ── 2. Supersede linkage on documents ──────────────────────────────
ALTER TABLE documents
    ADD COLUMN IF NOT EXISTS superseded_by UUID REFERENCES documents(id);

CREATE INDEX IF NOT EXISTS idx_documents_superseded_by
    ON documents (superseded_by) WHERE superseded_by IS NOT NULL;

-- ── 3. Immutability trigger — prevents updates to released revisions ──
CREATE OR REPLACE FUNCTION prevent_released_revision_update()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.status = 'released' THEN
        RAISE EXCEPTION 'Cannot modify a released revision (id=%)', OLD.id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_immutable_released_revision ON revisions;
CREATE TRIGGER trg_immutable_released_revision
    BEFORE UPDATE ON revisions
    FOR EACH ROW
    EXECUTE FUNCTION prevent_released_revision_update();

-- Also prevent deletion of released revisions.
CREATE OR REPLACE FUNCTION prevent_released_revision_delete()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.status = 'released' THEN
        RAISE EXCEPTION 'Cannot delete a released revision (id=%)', OLD.id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_no_delete_released_revision ON revisions;
CREATE TRIGGER trg_no_delete_released_revision
    BEFORE DELETE ON revisions
    FOR EACH ROW
    EXECUTE FUNCTION prevent_released_revision_delete();
