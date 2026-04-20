-- Integrations: push-attempt ledger for sync operations
--
-- Each row records one outbound push intent from platform → provider.
-- Statuses:
--   accepted        — intent recorded, not yet dispatched
--   inflight        — HTTP call in flight to provider
--   succeeded       — provider accepted the payload
--   failed          — provider rejected (4xx/5xx); deterministic, will not retry
--   unknown_failure — transport-level ambiguity (timeout, disconnect before response)
--                     watchdog will reconcile or escalate these rows
--
-- Partial unique index prevents duplicate concurrent intents for the same
-- (app, provider, entity, operation, authority version, request fingerprint)
-- while in accepted/inflight/succeeded states.  failed and unknown_failure rows
-- are excluded so a failed push can be retried with a fresh attempt row.
--
-- Watchdog scan index on (status, started_at) WHERE status = 'inflight' lets
-- the watchdog efficiently find stale in-flight rows without a full table scan.

CREATE TABLE integrations_sync_push_attempts (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id              TEXT        NOT NULL,
    provider            TEXT        NOT NULL,
    entity_type         TEXT        NOT NULL,
    entity_id           TEXT        NOT NULL,
    operation           TEXT        NOT NULL
        CHECK (operation IN ('create', 'update', 'delete')),
    authority_version   BIGINT      NOT NULL,
    request_fingerprint TEXT        NOT NULL,
    status              TEXT        NOT NULL DEFAULT 'accepted'
        CHECK (status IN ('accepted', 'inflight', 'succeeded', 'failed', 'unknown_failure')),
    error_message       TEXT,
    started_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at        TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Blocks duplicate in-flight or successful pushes for the same intent.
-- 'inflight' is included so an in-progress push cannot be double-dispatched.
CREATE UNIQUE INDEX integrations_sync_push_attempts_intent_unique
    ON integrations_sync_push_attempts (
        app_id, provider, entity_type, entity_id,
        operation, authority_version, request_fingerprint
    )
    WHERE status IN ('accepted', 'inflight', 'succeeded');

-- Watchdog: efficiently scan for stale inflight attempts by age.
CREATE INDEX integrations_sync_push_attempts_inflight_scan_idx
    ON integrations_sync_push_attempts (status, started_at)
    WHERE status = 'inflight';

-- General tenant lookup.
CREATE INDEX integrations_sync_push_attempts_app_idx
    ON integrations_sync_push_attempts (app_id, provider, entity_type);
