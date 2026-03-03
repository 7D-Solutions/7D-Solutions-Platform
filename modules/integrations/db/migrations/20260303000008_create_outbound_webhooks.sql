-- Outbound Webhook Management
--
-- Tenant-scoped outbound webhook subscriptions. When platform events fire,
-- the dispatcher finds matching webhooks by tenant_id + event_types and
-- delivers signed payloads to the configured URL.
--
-- signing_secret_hash: SHA-256 hex of the HMAC signing secret. Raw secret
--                      returned once at creation time, never stored.
--
-- event_types: JSONB array of subscribed event type strings.
--              Empty array = wildcard (subscribe to all).

CREATE TABLE integrations_outbound_webhooks (
    id                  UUID        NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY,
    tenant_id           TEXT        NOT NULL,
    url                 TEXT        NOT NULL,
    event_types         JSONB       NOT NULL DEFAULT '[]'::JSONB,
    signing_secret_hash TEXT        NOT NULL,
    status              TEXT        NOT NULL DEFAULT 'active'
                                    CHECK (status IN ('active', 'paused', 'disabled')),
    idempotency_key     TEXT,
    description         TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at          TIMESTAMPTZ
);

-- Idempotent creation: one webhook per (tenant_id, idempotency_key)
CREATE UNIQUE INDEX idx_outbound_webhooks_idempotency
    ON integrations_outbound_webhooks (tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL AND deleted_at IS NULL;

-- Dispatcher lookup: active webhooks per tenant
CREATE INDEX idx_outbound_webhooks_tenant_active
    ON integrations_outbound_webhooks (tenant_id)
    WHERE status = 'active' AND deleted_at IS NULL;

-- Admin listing per tenant
CREATE INDEX idx_outbound_webhooks_tenant_created
    ON integrations_outbound_webhooks (tenant_id, created_at DESC);

-- ============================================================================
-- Delivery audit log
-- ============================================================================
--
-- Every outbound delivery attempt is logged here. Success or failure,
-- each attempt gets a row. Retry metadata tracks exponential backoff.

CREATE TABLE integrations_outbound_webhook_deliveries (
    id              UUID        NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY,
    webhook_id      UUID        NOT NULL REFERENCES integrations_outbound_webhooks(id),
    tenant_id       TEXT        NOT NULL,
    event_type      TEXT        NOT NULL,
    payload         JSONB       NOT NULL,
    status_code     INT,
    response_body   TEXT,
    error_message   TEXT,
    attempt_number  INT         NOT NULL DEFAULT 1,
    next_retry_at   TIMESTAMPTZ,
    delivered_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Query deliveries for a specific webhook
CREATE INDEX idx_outbound_deliveries_webhook
    ON integrations_outbound_webhook_deliveries (webhook_id, created_at DESC);

-- Query deliveries by tenant
CREATE INDEX idx_outbound_deliveries_tenant
    ON integrations_outbound_webhook_deliveries (tenant_id, created_at DESC);
