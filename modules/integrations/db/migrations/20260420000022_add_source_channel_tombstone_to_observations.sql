-- Add source_channel and is_tombstone to the observations ledger.
--
-- source_channel: which ingestion path produced this observation
--   ('cdc', 'full_resync', 'webhook', 'unknown').
--
-- is_tombstone: TRUE when the provider reports the entity as deleted.
--   CDC responses include entities with status="Deleted"; full-resync never
--   returns deleted entities, so this will always be FALSE there.

ALTER TABLE integrations_sync_observations
    ADD COLUMN source_channel TEXT    NOT NULL DEFAULT 'unknown',
    ADD COLUMN is_tombstone   BOOLEAN NOT NULL DEFAULT FALSE;

-- Partial index: only tombstones need fast lookup (typically a tiny fraction).
CREATE INDEX integrations_sync_observations_tombstone_idx
    ON integrations_sync_observations (app_id, provider, entity_type, is_tombstone)
    WHERE is_tombstone;
