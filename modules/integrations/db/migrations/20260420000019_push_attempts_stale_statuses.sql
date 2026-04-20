-- Extend integrations_sync_push_attempts with two new terminal statuses:
--
--   superseded                      — pre-call: authority changed before HTTP dispatch;
--                                     no write was made to the external provider.
--   completed_under_stale_authority — post-call: HTTP call completed but authority
--                                     changed while the call was in flight; reconciliation
--                                     is required to auto-close (equivalent) or open conflict.
--
-- Neither status joins the dedup unique index: both are terminal states from which
-- a fresh attempt (stamped with the current authority_version) is permitted.
-- For completed_under_stale_authority the reconciliation conflict row (if opened)
-- serves as the guard against premature re-attempts at the service layer.

ALTER TABLE integrations_sync_push_attempts
    DROP CONSTRAINT integrations_sync_push_attempts_status_check;

ALTER TABLE integrations_sync_push_attempts
    ADD CONSTRAINT integrations_sync_push_attempts_status_check
    CHECK (status IN (
        'accepted', 'inflight', 'succeeded', 'failed',
        'unknown_failure', 'superseded', 'completed_under_stale_authority'
    ));
