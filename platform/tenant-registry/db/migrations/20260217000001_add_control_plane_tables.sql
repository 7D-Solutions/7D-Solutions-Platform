-- Control-Plane Provisioning Tables
-- Adds idempotency tracking and outbox for the control-plane HTTP API
-- These tables support the POST /api/control/tenants endpoint

-- ============================================================
-- EXTEND TENANT STATUS CONSTRAINT
-- ============================================================

-- Add 'pending' and 'failed' to the allowed tenant status values.
-- 'pending': tenant record created, provisioning not yet started.
-- 'failed':  provisioning failed; tenant not operational.
ALTER TABLE tenants
    DROP CONSTRAINT IF EXISTS tenants_status_check;

ALTER TABLE tenants
    ADD CONSTRAINT tenants_status_check
    CHECK (status IN ('pending', 'provisioning', 'active', 'failed', 'suspended', 'deleted'));

-- ============================================================
-- PROVISIONING REQUESTS (Idempotency)
-- ============================================================

-- Tracks idempotency keys for tenant provisioning requests.
-- Guarantees at-most-once creation per key.
CREATE TABLE IF NOT EXISTS provisioning_requests (
    idempotency_key VARCHAR(255) PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
    environment VARCHAR(20) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

COMMENT ON TABLE provisioning_requests IS 'Idempotency registry for control-plane tenant provisioning requests';
COMMENT ON COLUMN provisioning_requests.idempotency_key IS 'Caller-supplied idempotency key (e.g., UUID or hash)';
COMMENT ON COLUMN provisioning_requests.tenant_id IS 'Tenant created for this request';

-- ============================================================
-- PROVISIONING OUTBOX
-- ============================================================

-- Append-only outbox for provisioning lifecycle events.
-- Events: tenant.provisioning_started, tenant.provisioned, tenant.provisioning_failed
-- Published by the outbox relay; published_at is null until delivered.
CREATE TABLE IF NOT EXISTS provisioning_outbox (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
    event_type VARCHAR(100) NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    published_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS provisioning_outbox_unpublished
    ON provisioning_outbox(created_at)
    WHERE published_at IS NULL;

COMMENT ON TABLE provisioning_outbox IS 'Append-only outbox for provisioning lifecycle events';
COMMENT ON COLUMN provisioning_outbox.event_type IS 'e.g. tenant.provisioning_started, tenant.provisioned, tenant.provisioning_failed';
COMMENT ON COLUMN provisioning_outbox.published_at IS 'Set when the event has been delivered to the event bus; null means pending';
