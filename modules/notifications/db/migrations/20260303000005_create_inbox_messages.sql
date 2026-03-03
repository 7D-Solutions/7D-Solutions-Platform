-- Phase 57b N4: per-user in-app inbox.
--
-- Each row represents one inbox message for a specific user, linked to the
-- underlying scheduled_notification (delivery).  The composite unique
-- constraint on (notification_id, user_id) guarantees at most one inbox
-- item per logical notification delivery per recipient, even under retries
-- or event replays.
--
-- ROLLBACK: DROP TABLE IF EXISTS inbox_messages;

CREATE TABLE IF NOT EXISTS inbox_messages (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    user_id          TEXT NOT NULL,
    notification_id  UUID NOT NULL REFERENCES scheduled_notifications(id) ON DELETE CASCADE,
    title            TEXT NOT NULL,
    body             TEXT,
    category         TEXT,
    is_read          BOOLEAN NOT NULL DEFAULT FALSE,
    is_dismissed     BOOLEAN NOT NULL DEFAULT FALSE,
    read_at          TIMESTAMPTZ,
    dismissed_at     TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_inbox_notification_user UNIQUE (notification_id, user_id)
);

-- Primary listing query: tenant + user, ordered by newest first
CREATE INDEX idx_inbox_tenant_user
    ON inbox_messages (tenant_id, user_id, created_at DESC);

-- Unread-only filter
CREATE INDEX idx_inbox_unread
    ON inbox_messages (tenant_id, user_id, created_at DESC)
    WHERE is_read = FALSE AND is_dismissed = FALSE;

-- Tenant-level admin queries
CREATE INDEX idx_inbox_tenant
    ON inbox_messages (tenant_id);
