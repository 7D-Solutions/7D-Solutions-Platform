-- Add idempotency_key for retry-safe checkout session creation.
-- Re-add client_secret (dropped in 20260227000001) — needed to return
-- the secret on idempotent retries without re-calling the PSP.
--
-- Effective key logic (handler-side):
--   explicit idempotency_key if provided, else invoice_id as natural key.
-- UNIQUE(tenant_id, idempotency_key) prevents duplicate payment intents.

ALTER TABLE checkout_sessions ADD COLUMN IF NOT EXISTS idempotency_key TEXT;
ALTER TABLE checkout_sessions ADD COLUMN IF NOT EXISTS client_secret TEXT;

-- Backfill existing rows with their UUID so NOT NULL + UNIQUE are safe
UPDATE checkout_sessions SET idempotency_key = id::text WHERE idempotency_key IS NULL;

ALTER TABLE checkout_sessions ALTER COLUMN idempotency_key SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_checkout_sessions_tenant_idem_key
    ON checkout_sessions (tenant_id, idempotency_key);
