-- Escalation rules: configurable escalation chains for unacknowledged notifications.
--
-- If a notification is not acknowledged within `timeout_secs`, escalation sends
-- are created targeting the next recipient/channel in the chain.
--
-- ROLLBACK:
--   DROP TABLE IF EXISTS escalation_sends;
--   DROP TABLE IF EXISTS escalation_rules;
--   ALTER TABLE scheduled_notifications DROP COLUMN IF EXISTS acknowledged_at;

-- 1. Add acknowledged_at to scheduled_notifications so we can detect unacknowledged sends.
ALTER TABLE scheduled_notifications
    ADD COLUMN IF NOT EXISTS acknowledged_at TIMESTAMPTZ;

-- 2. Escalation rules: tenant-scoped, one per (tenant, source_notification_type, level).
CREATE TABLE IF NOT EXISTS escalation_rules (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    source_notification_type TEXT NOT NULL,
    level           INT NOT NULL DEFAULT 1,
    timeout_secs    INT NOT NULL,
    target_channel  TEXT NOT NULL,
    target_recipient TEXT NOT NULL,
    priority        TEXT NOT NULL DEFAULT 'high',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_escalation_rule UNIQUE (tenant_id, source_notification_type, level)
);

CREATE INDEX IF NOT EXISTS idx_escalation_rules_tenant
    ON escalation_rules (tenant_id);

CREATE INDEX IF NOT EXISTS idx_escalation_rules_lookup
    ON escalation_rules (tenant_id, source_notification_type, level);

-- 3. Escalation sends: tracks which escalations have been fired, with idempotency.
CREATE TABLE IF NOT EXISTS escalation_sends (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    source_notification_id UUID NOT NULL,
    escalation_rule_id  UUID NOT NULL REFERENCES escalation_rules(id),
    level               INT NOT NULL,
    target_channel      TEXT NOT NULL,
    target_recipient    TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Idempotency: one escalation send per (source_notification, rule) pair.
    CONSTRAINT uq_escalation_send UNIQUE (source_notification_id, escalation_rule_id)
);

CREATE INDEX IF NOT EXISTS idx_escalation_sends_source
    ON escalation_sends (source_notification_id);

CREATE INDEX IF NOT EXISTS idx_escalation_sends_tenant
    ON escalation_sends (tenant_id);
