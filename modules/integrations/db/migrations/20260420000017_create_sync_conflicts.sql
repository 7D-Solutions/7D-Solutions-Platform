-- Sync conflict records: each row represents a detected divergence between
-- the platform's internal state and the external integration for an entity.
--
-- class semantics:
--   creation   — entity exists on one side but not the other
--   edit       — entity exists on both sides but values differ
--   deletion   — entity was deleted on one side; still present on the other
--
-- status transitions (application-enforced; DB guards the no-skip rule):
--   pending → resolved | ignored | unresolvable
--   Transitions back to pending or lateral moves are not permitted.

CREATE TABLE integrations_sync_conflicts (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id          TEXT        NOT NULL,
    provider        TEXT        NOT NULL,
    entity_type     TEXT        NOT NULL,
    entity_id       TEXT        NOT NULL,
    conflict_class  TEXT        NOT NULL
        CHECK (conflict_class IN ('creation', 'edit', 'deletion')),
    status          TEXT        NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'resolved', 'ignored', 'unresolvable')),

    -- Source attribution
    detected_by     TEXT        NOT NULL,
    detected_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Divergent snapshots — required for creation/edit; nullable for deletion
    -- Application enforces the 256 KB cap on each column before INSERT.
    internal_value  JSONB,
    external_value  JSONB,

    -- Set when status transitions to 'resolved'; must not be null at that point
    internal_id     TEXT,

    -- Resolution metadata
    resolved_by     TEXT,
    resolved_at     TIMESTAMPTZ,
    resolution_note TEXT,

    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- creation/edit require both value snapshots; deletion does not
    CONSTRAINT sync_conflicts_values_required
        CHECK (
            conflict_class = 'deletion'
            OR (internal_value IS NOT NULL AND external_value IS NOT NULL)
        ),

    -- resolved status requires a reconciled internal_id
    CONSTRAINT sync_conflicts_resolved_needs_internal_id
        CHECK (status != 'resolved' OR internal_id IS NOT NULL)
);

-- Hot path: list pending conflicts for a given app+provider+entity_type
CREATE INDEX integrations_sync_conflicts_pending_idx
    ON integrations_sync_conflicts (app_id, provider, entity_type)
    WHERE status = 'pending';

-- Entity-level lookup (used by detector and resolve handler)
CREATE INDEX integrations_sync_conflicts_entity_idx
    ON integrations_sync_conflicts (app_id, entity_type, entity_id);

-- Chronological retrieval per app (paged list endpoints)
CREATE INDEX integrations_sync_conflicts_app_created_idx
    ON integrations_sync_conflicts (app_id, created_at DESC);
