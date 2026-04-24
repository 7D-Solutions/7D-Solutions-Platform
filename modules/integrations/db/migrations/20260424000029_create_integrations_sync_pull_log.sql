-- Track manual per-tenant sync-pull requests; partial unique index enforces one
-- inflight pull per tenant at a time (released when status transitions to terminal).
CREATE TABLE integrations_sync_pull_log (
  id             UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
  app_id         TEXT        NOT NULL,
  entity_type    TEXT        NOT NULL,
  triggered_by   TEXT        NOT NULL,
  started_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  completed_at   TIMESTAMPTZ,
  status         TEXT        NOT NULL CHECK (status IN ('inflight', 'complete', 'failed')),
  error          TEXT
);

CREATE UNIQUE INDEX integrations_sync_pull_log_inflight
  ON integrations_sync_pull_log (app_id, entity_type)
  WHERE status = 'inflight';
