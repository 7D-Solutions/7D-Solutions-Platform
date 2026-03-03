-- Phase 57 N1a: first-class delivery status model on scheduled_notifications.
ALTER TABLE scheduled_notifications
  ADD COLUMN IF NOT EXISTS attempted_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS sent_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS failed_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS dead_lettered_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS last_error TEXT;

-- Normalize in-flight state naming for the new model.
UPDATE scheduled_notifications
SET status = 'attempting'
WHERE status = 'claimed';

CREATE INDEX IF NOT EXISTS idx_sn_status_deliver_at
  ON scheduled_notifications (status, deliver_at);
