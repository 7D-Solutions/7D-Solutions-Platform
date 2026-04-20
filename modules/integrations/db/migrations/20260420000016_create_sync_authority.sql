-- Integrations: per-(app_id, provider, entity_type) sync authority record
--
-- Tracks which side (platform or external) is authoritative for a given
-- entity type within a connected integration. authority_version is
-- monotonic — it only ever increments, never resets. The flip service
-- (bd-y7np7) will use an advisory lock + bump_version() to safely switch
-- authoritative_side without race conditions.

CREATE TABLE integrations_sync_authority (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id              TEXT        NOT NULL,
    provider            TEXT        NOT NULL,
    entity_type         TEXT        NOT NULL,
    authoritative_side  TEXT        NOT NULL
        CHECK (authoritative_side IN ('platform', 'external'))
        DEFAULT 'platform',
    authority_version   BIGINT      NOT NULL DEFAULT 1
        CHECK (authority_version >= 1),
    last_flipped_by     TEXT,
    last_flipped_at     TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT integrations_sync_authority_unique
        UNIQUE (app_id, provider, entity_type)
);

CREATE INDEX integrations_sync_authority_app_provider_idx
    ON integrations_sync_authority (app_id, provider);
