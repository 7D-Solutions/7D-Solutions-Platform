-- Item Change History: immutable audit trail for item master changes (bd-1nj81).
--
-- Captures who changed what, when, and why for item revisions and
-- policy changes. Each row is a point-in-time snapshot of the change
-- with a structured JSON diff of before/after values.
--
-- Invariants:
-- - Rows are immutable once written (no UPDATE/DELETE in application code)
-- - idempotency_key prevents duplicate history rows under retries
-- - tenant_id scoping prevents cross-tenant visibility
-- - Entries are always ordered chronologically (created_at ASC, id ASC)

CREATE TABLE item_change_history (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    item_id           UUID NOT NULL REFERENCES items(id),
    revision_id       UUID REFERENCES item_revisions(id),
    change_type       TEXT NOT NULL,
    actor_id          TEXT NOT NULL,
    diff              JSONB NOT NULL,
    reason            TEXT,
    idempotency_key   TEXT,
    created_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Prevent duplicate history entries under retry
    CONSTRAINT item_change_history_tenant_idemp_unique
        UNIQUE (tenant_id, idempotency_key),

    -- change_type must be a known value
    CONSTRAINT item_change_history_change_type_check
        CHECK (change_type IN ('revision_created', 'revision_activated', 'policy_updated'))
);

-- Primary query: list all changes for a (tenant, item) in chronological order
CREATE INDEX idx_item_change_history_tenant_item
    ON item_change_history(tenant_id, item_id, created_at ASC, id ASC);

-- Query by revision
CREATE INDEX idx_item_change_history_revision
    ON item_change_history(tenant_id, revision_id)
    WHERE revision_id IS NOT NULL;
