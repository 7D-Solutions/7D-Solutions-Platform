-- Server-computed deterministic idempotency key for bulk conflict resolutions.
-- Stores the sha256(conflict_id:action:authority_version) key set by the server
-- during bulk resolve operations.  NULL for conflicts resolved via single-item
-- endpoint (which pre-dates this mechanism) or by automated handlers.
ALTER TABLE integrations_sync_conflicts
    ADD COLUMN resolution_idempotency_key TEXT;

CREATE INDEX integrations_sync_conflicts_idem_key_idx
    ON integrations_sync_conflicts (app_id, resolution_idempotency_key)
    WHERE resolution_idempotency_key IS NOT NULL;
