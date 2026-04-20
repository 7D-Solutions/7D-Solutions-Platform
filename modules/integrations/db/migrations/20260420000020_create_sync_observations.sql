-- Sync observation ledger: one row per observed provider entity state.
--
-- An observation captures a point-in-time snapshot of an entity as seen from
-- a provider (webhook, CDC, poll).  Deduplication is enforced via the unique
-- constraint on (app_id, provider, entity_type, entity_id, fingerprint).
--
-- Timestamp invariant:
--   last_updated_time MUST be truncated to millisecond precision before insert.
--   The DB-level CHECK enforces this; the application layer is responsible for
--   calling date_trunc / truncating via chrono before binding.  This prevents
--   silent equality failures when correlating ISO-8601 strings that carry
--   sub-millisecond precision (e.g., "2024-01-01T12:00:00.0001234Z").
--
-- Fingerprint fallback rule (application-enforced):
--   1. provider sync_token present  → "st:<sync_token>"
--   2. last_updated_time present    → "ts:<epoch_millis>"
--   3. neither present              → "ph:<sha256(raw_payload)>"
--   Fallback prevents fingerprint collapse (all-null → all rows same key) when
--   the provider omits versioning metadata.
--
-- comparable_hash: SHA-256 of the canonical comparable projection fields, used
--   for equality-based correlation without re-parsing the full payload.
--
-- projection_version: monotone integer; bump when the comparable_hash or
--   fingerprint computation logic changes.  Allows stale hashes to be
--   detected and recomputed without touching the unique key.

CREATE TABLE integrations_sync_observations (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id              TEXT        NOT NULL,
    provider            TEXT        NOT NULL,
    entity_type         TEXT        NOT NULL,
    entity_id           TEXT        NOT NULL,

    -- Deduplication key component (one of: st:<token>, ts:<ms>, ph:<hash>)
    fingerprint         TEXT        NOT NULL,

    -- Normalized provider timestamp, always millisecond-truncated UTC.
    -- The CHECK guard here is belt-and-suspenders; app layer truncates first.
    last_updated_time   TIMESTAMPTZ NOT NULL
        CHECK (date_trunc('milliseconds', last_updated_time) = last_updated_time),

    -- Stable equality hash of the comparable projection.
    comparable_hash     TEXT        NOT NULL,

    -- Projection schema version; bump when hash logic changes.
    projection_version  INT         NOT NULL DEFAULT 1,

    -- Raw provider payload kept verbatim for audit; NOT used for correlation.
    raw_payload         JSONB       NOT NULL,

    observed_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Deduplicate: same entity state from same provider seen only once.
    CONSTRAINT integrations_sync_observations_unique
        UNIQUE (app_id, provider, entity_type, entity_id, fingerprint)
);

-- Hot path: look up latest observation for a specific entity.
CREATE INDEX integrations_sync_observations_entity_idx
    ON integrations_sync_observations (app_id, provider, entity_type, entity_id);

-- High-watermark / CDC queries: scan forward from a known timestamp per provider.
CREATE INDEX integrations_sync_observations_watermark_idx
    ON integrations_sync_observations (app_id, provider, last_updated_time DESC);

-- Hash-based correlation: find rows by comparable_hash (marker equality checks).
CREATE INDEX integrations_sync_observations_comparable_hash_idx
    ON integrations_sync_observations (app_id, provider, entity_type, comparable_hash);
