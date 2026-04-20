-- Integrations: per-(app_id, provider, job_name) sync job health row
--
-- Non-push workers (CDC poll, token refresh) upsert one row per tick so
-- operators can query "is sync healthy/degraded/failing?" per tenant without
-- a heavyweight health subsystem.
--
-- failure_streak resets to 0 on any success; increments on each failure.
-- An operator sees at a glance how many consecutive failures have occurred.

CREATE TABLE integrations_sync_jobs (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id              TEXT        NOT NULL,
    provider            TEXT        NOT NULL,
    job_name            TEXT        NOT NULL,
    last_success_at     TIMESTAMPTZ,
    last_failure_at     TIMESTAMPTZ,
    failure_streak      INT         NOT NULL DEFAULT 0
        CHECK (failure_streak >= 0),
    last_error          TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT integrations_sync_jobs_unique
        UNIQUE (app_id, provider, job_name)
);

CREATE INDEX integrations_sync_jobs_app_provider_idx
    ON integrations_sync_jobs (app_id, provider);
