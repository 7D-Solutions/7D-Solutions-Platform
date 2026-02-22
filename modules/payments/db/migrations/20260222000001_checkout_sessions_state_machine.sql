-- Expand checkout_session state machine
-- Old status values: pending | succeeded | failed | cancelled
-- New state machine: created → presented → completed | failed | canceled | expired
--
-- Transitions:
--   created  → presented  (hosted page load — idempotent)
--   created  → completed  (webhook: payment_intent.succeeded — page never visited)
--   created  → failed     (webhook: payment_intent.payment_failed)
--   created  → canceled   (webhook: payment_intent.canceled)
--   presented → completed (webhook: payment_intent.succeeded)
--   presented → failed    (webhook: payment_intent.payment_failed)
--   presented → canceled  (webhook: payment_intent.canceled)
--   any non-terminal → expired (future: scheduled expiry job)
--
-- Terminal states: completed | failed | canceled | expired

-- Migrate existing data before adding the check constraint
UPDATE checkout_sessions SET status = 'created'   WHERE status = 'pending';
UPDATE checkout_sessions SET status = 'completed'  WHERE status = 'succeeded';
UPDATE checkout_sessions SET status = 'canceled'   WHERE status = 'cancelled';

-- Change column default to 'created'
ALTER TABLE checkout_sessions ALTER COLUMN status SET DEFAULT 'created';

-- Add presented_at timestamp (set when hosted page first loads)
ALTER TABLE checkout_sessions ADD COLUMN IF NOT EXISTS presented_at TIMESTAMPTZ;

-- Enforce valid status values
ALTER TABLE checkout_sessions
    ADD CONSTRAINT checkout_sessions_status_check
    CHECK (status IN ('created', 'presented', 'completed', 'failed', 'canceled', 'expired'));
