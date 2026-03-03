-- Delivery attempt journal for idempotent outbound notification sends.
-- Ensures duplicate idempotency keys do not trigger duplicate side effects.
CREATE TABLE IF NOT EXISTS notification_delivery_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    notification_id UUID NOT NULL REFERENCES scheduled_notifications(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL UNIQUE,
    attempt_no INT NOT NULL,
    status TEXT NOT NULL, -- in_progress|succeeded|failed_retryable|failed_permanent
    provider_message_id TEXT,
    error_class TEXT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_notification_delivery_attempts_notification
    ON notification_delivery_attempts(notification_id);
