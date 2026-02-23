CREATE TABLE scheduled_notifications (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    recipient_ref   TEXT NOT NULL,
    channel         TEXT NOT NULL,
    template_key    TEXT NOT NULL,
    payload_json    JSONB NOT NULL,
    deliver_at      TIMESTAMPTZ NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    retry_count     INT NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_sn_deliver_at_status ON scheduled_notifications (deliver_at, status) WHERE status = 'pending';
