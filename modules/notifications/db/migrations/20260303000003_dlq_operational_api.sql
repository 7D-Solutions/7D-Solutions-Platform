-- Phase 57 N1b: DLQ operational API support.
-- Adds abandoned_at timestamp, replay_generation counter for idempotent replay.

ALTER TABLE scheduled_notifications
    ADD COLUMN IF NOT EXISTS abandoned_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS replay_generation INT NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_sn_dead_lettered
    ON scheduled_notifications (dead_lettered_at)
    WHERE status = 'dead_lettered';

CREATE INDEX IF NOT EXISTS idx_sn_abandoned
    ON scheduled_notifications (abandoned_at)
    WHERE status = 'abandoned';
