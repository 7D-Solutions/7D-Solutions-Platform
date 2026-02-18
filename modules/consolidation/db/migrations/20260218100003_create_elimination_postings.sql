-- Consolidation Module: Elimination Posting Idempotency
-- bd-23sw: tracks posted elimination journals for exactly-once semantics
--
-- Design principles:
--   - Keyed by (group_id, period_id, idempotency_key) for exactly-once posting
--   - Stores GL journal entry IDs for audit trail
--   - Tables prefixed csl_ to avoid clashes with source-module schemas

CREATE TABLE csl_elimination_postings (
    id                UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    group_id          UUID        NOT NULL REFERENCES csl_groups(id) ON DELETE CASCADE,
    period_id         UUID        NOT NULL,
    idempotency_key   TEXT        NOT NULL,
    journal_entry_ids JSONB       NOT NULL DEFAULT '[]',
    suggestion_count  INT         NOT NULL DEFAULT 0,
    total_amount_minor BIGINT     NOT NULL DEFAULT 0,
    posted_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT csl_elimination_postings_unique
        UNIQUE (group_id, period_id, idempotency_key)
);

CREATE INDEX idx_csl_elim_postings_group_period
    ON csl_elimination_postings (group_id, period_id);

COMMENT ON TABLE csl_elimination_postings IS
    'Elimination posting log: ensures exactly-once posting of elimination journals per group+period.';
