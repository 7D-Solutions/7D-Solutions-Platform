-- Remove client_secret from checkout_sessions.
-- The server never reads it after session creation — only the browser needs it,
-- and it gets the value from the POST response (pass-through from Tilled API).
-- Storing it is unnecessary and a security concern.

ALTER TABLE checkout_sessions DROP COLUMN IF EXISTS client_secret;
