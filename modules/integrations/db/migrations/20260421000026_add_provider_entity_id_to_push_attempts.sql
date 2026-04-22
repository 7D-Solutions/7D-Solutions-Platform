-- Stores the provider's entity id (e.g. QBO Customer.Id) on successful CREATE pushes.
-- NULL for UPDATE operations where both sides share the same entity id.
-- Enables find_attempt_by_markers to correlate observations that arrive using
-- the provider's id namespace instead of the platform's id namespace.
ALTER TABLE integrations_sync_push_attempts
    ADD COLUMN IF NOT EXISTS provider_entity_id TEXT NULL;
