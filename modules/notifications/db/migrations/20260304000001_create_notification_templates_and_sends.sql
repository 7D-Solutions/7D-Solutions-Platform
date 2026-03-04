-- Phase 67: Notification templates (versioned) + notification sends + delivery receipts.
-- Provides compliance-grade notification infrastructure with full audit trail.

-- ── Versioned notification templates ──────────────────────────────────────────
-- Each template_key + tenant_id can have multiple versions. Publishing always
-- creates a new version. The latest version is resolved via MAX(version).

CREATE TABLE IF NOT EXISTS notification_templates (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    template_key TEXT NOT NULL,
    version INT NOT NULL DEFAULT 1,
    channel TEXT NOT NULL,            -- email | sms | inbox | webhook
    subject TEXT NOT NULL,
    body TEXT NOT NULL,
    required_vars JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by TEXT,

    UNIQUE (tenant_id, template_key, version)
);

CREATE INDEX idx_notification_templates_lookup
    ON notification_templates(tenant_id, template_key, version DESC);

-- ── Notification sends ────────────────────────────────────────────────────────
-- Each send request is recorded with the template version used and a hash of the
-- rendered content for compliance proof.

CREATE TABLE IF NOT EXISTS notification_sends (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    template_key TEXT NOT NULL,
    template_version INT NOT NULL,
    channel TEXT NOT NULL,
    recipients JSONB NOT NULL DEFAULT '[]'::jsonb,
    payload_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    correlation_id TEXT,
    causation_id TEXT,
    rendered_hash TEXT,               -- SHA-256 of rendered content
    status TEXT NOT NULL DEFAULT 'pending',  -- pending | delivering | delivered | partially_failed | failed
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_notification_sends_tenant ON notification_sends(tenant_id);
CREATE INDEX idx_notification_sends_correlation ON notification_sends(correlation_id)
    WHERE correlation_id IS NOT NULL;
CREATE INDEX idx_notification_sends_status ON notification_sends(tenant_id, status);

-- ── Delivery receipts ─────────────────────────────────────────────────────────
-- One receipt per recipient per send attempt. Provides evidence-grade proof of
-- who was notified, when, via which channel, and the outcome.

CREATE TABLE IF NOT EXISTS delivery_receipts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    send_id UUID NOT NULL REFERENCES notification_sends(id) ON DELETE CASCADE,
    recipient TEXT NOT NULL,
    channel TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',  -- pending | attempted | succeeded | failed | dlq
    provider_id TEXT,                 -- external provider message reference
    attempt_count INT NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    succeeded_at TIMESTAMPTZ,
    failed_at TIMESTAMPTZ,
    error_class TEXT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_delivery_receipts_send ON delivery_receipts(send_id);
CREATE INDEX idx_delivery_receipts_tenant ON delivery_receipts(tenant_id);
CREATE INDEX idx_delivery_receipts_correlation
    ON delivery_receipts(tenant_id, created_at DESC);
CREATE INDEX idx_delivery_receipts_recipient
    ON delivery_receipts(tenant_id, recipient);
