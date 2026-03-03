-- DOC4: controlled distribution + delivery confirmation.

CREATE TABLE IF NOT EXISTS document_distributions (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           UUID NOT NULL,
    document_id         UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    revision_id         UUID,
    recipient_ref       TEXT NOT NULL,
    channel             VARCHAR(32) NOT NULL,
    template_key        TEXT NOT NULL,
    payload_json        JSONB NOT NULL DEFAULT '{}'::jsonb,
    status              VARCHAR(32) NOT NULL DEFAULT 'pending',
    provider_message_id TEXT,
    requested_by        UUID NOT NULL,
    requested_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at             TIMESTAMPTZ,
    delivered_at        TIMESTAMPTZ,
    failed_at           TIMESTAMPTZ,
    failure_reason      TEXT,
    idempotency_key     TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_distribution_idem UNIQUE (tenant_id, idempotency_key),
    CONSTRAINT chk_distribution_status
      CHECK (status IN ('pending', 'sent', 'delivered', 'failed', 'ignored'))
);

CREATE INDEX idx_distributions_doc ON document_distributions (tenant_id, document_id, created_at DESC);
CREATE INDEX idx_distributions_status ON document_distributions (tenant_id, status, requested_at);

CREATE TABLE IF NOT EXISTS document_distribution_status_log (
    id                  BIGSERIAL PRIMARY KEY,
    distribution_id     UUID NOT NULL REFERENCES document_distributions(id) ON DELETE CASCADE,
    tenant_id           UUID NOT NULL,
    previous_status     VARCHAR(32),
    new_status          VARCHAR(32) NOT NULL,
    idempotency_key     TEXT NOT NULL,
    notification_event_id UUID,
    payload_json        JSONB NOT NULL DEFAULT '{}'::jsonb,
    changed_by          UUID,
    changed_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_distribution_status_idem UNIQUE (tenant_id, idempotency_key),
    CONSTRAINT chk_distribution_log_status
      CHECK (new_status IN ('pending', 'sent', 'delivered', 'failed', 'ignored'))
);

CREATE INDEX idx_distribution_status_log_dist ON document_distribution_status_log(distribution_id, changed_at DESC);
CREATE INDEX idx_distribution_status_log_event ON document_distribution_status_log(notification_event_id) WHERE notification_event_id IS NOT NULL;
