-- Broadcast announcements: tenant-scoped broadcasts to all users or role-based audiences.
--
-- A broadcast represents a high-fanout announcement. The `broadcasts` table stores
-- the announcement itself, and `broadcast_recipients` stores the individual delivery
-- records created during fan-out.
--
-- The `idempotency_key` on broadcasts guarantees that retries or replays of the same
-- broadcast request do not create duplicate fan-outs (N^2 protection).
--
-- ROLLBACK:
--   DROP TABLE IF EXISTS broadcast_recipients;
--   DROP TABLE IF EXISTS broadcasts;

CREATE TABLE IF NOT EXISTS broadcasts (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    idempotency_key  TEXT NOT NULL,
    audience_type    TEXT NOT NULL CHECK (audience_type IN ('all_tenant', 'role')),
    audience_filter  TEXT,  -- role name when audience_type = 'role', NULL for all_tenant
    title            TEXT NOT NULL,
    body             TEXT,
    channel          TEXT NOT NULL DEFAULT 'in_app',
    status           TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'fan_out_complete', 'failed')),
    recipient_count  INT NOT NULL DEFAULT 0,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_broadcast_idempotency UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_broadcasts_tenant ON broadcasts (tenant_id, created_at DESC);
CREATE INDEX idx_broadcasts_status ON broadcasts (status) WHERE status = 'pending';

CREATE TABLE IF NOT EXISTS broadcast_recipients (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    broadcast_id   UUID NOT NULL REFERENCES broadcasts(id) ON DELETE CASCADE,
    tenant_id      TEXT NOT NULL,
    user_id        TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_broadcast_recipient UNIQUE (broadcast_id, user_id)
);

CREATE INDEX idx_broadcast_recipients_broadcast ON broadcast_recipients (broadcast_id);
CREATE INDEX idx_broadcast_recipients_tenant ON broadcast_recipients (tenant_id);
CREATE INDEX idx_broadcast_recipients_user ON broadcast_recipients (tenant_id, user_id);
