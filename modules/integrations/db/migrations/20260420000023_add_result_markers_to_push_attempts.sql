-- Integrations: add push-result markers to the push-attempt ledger.
--
-- These three columns record what the provider returned on a successful push
-- so that downstream conflict detection can compare the pushed state against
-- later CDC observations without re-reading the provider.
--
-- result_sync_token       — QBO SyncToken from the provider response.
-- result_last_updated_time — Millisecond-truncated UTC timestamp from the
--                            provider response MetaData.LastUpdatedTime.
--                            DB CHECK enforces the ms truncation invariant.
-- result_projection_hash  — Canonical fingerprint of the external_value body;
--                            used for equality correlation in conflict detection.
--
-- All three columns are NULL until the attempt reaches 'succeeded'; they remain
-- NULL for failed, unknown_failure, superseded, and stale-authority outcomes.

ALTER TABLE integrations_sync_push_attempts
    ADD COLUMN result_sync_token        TEXT,
    ADD COLUMN result_last_updated_time TIMESTAMPTZ
        CONSTRAINT push_attempts_result_lut_ms_precision
        CHECK (
            result_last_updated_time IS NULL
            OR date_trunc('milliseconds', result_last_updated_time) = result_last_updated_time
        ),
    ADD COLUMN result_projection_hash   TEXT;
