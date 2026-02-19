-- Integrations Module: Webhook Endpoint Configurations
--
-- Stores outbound webhook endpoint configurations. When platform events fire,
-- the webhook dispatcher consults this table to find active endpoints that
-- subscribe to the relevant event types.
--
-- secret_hash: HMAC signing secret stored as SHA-256 hex. Never store
--              the raw secret — only the hash is persisted here. The
--              raw secret is returned once at creation time via the API.
--
-- event_types: JSONB array of event type strings (e.g. ["invoice.created",
--              "payment.received"]). Empty array = subscribe to all events.

CREATE TABLE integrations_webhook_endpoints (
    id           UUID    NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY,
    app_id       TEXT    NOT NULL,
    name         TEXT    NOT NULL,          -- human-readable label
    url          TEXT    NOT NULL,          -- HTTPS delivery target
    secret_hash  TEXT    NOT NULL,          -- SHA-256 hex of HMAC signing secret
    event_types  JSONB   NOT NULL DEFAULT '[]'::JSONB,
    enabled      BOOLEAN NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    deleted_at   TIMESTAMP WITH TIME ZONE               -- soft-delete
);

-- Dispatcher scans enabled endpoints per app
CREATE INDEX idx_integrations_wh_endpoints_app_enabled
    ON integrations_webhook_endpoints(app_id)
    WHERE enabled = TRUE AND deleted_at IS NULL;

-- All endpoints for an app (admin listing)
CREATE INDEX idx_integrations_wh_endpoints_app
    ON integrations_webhook_endpoints(app_id, created_at DESC);
