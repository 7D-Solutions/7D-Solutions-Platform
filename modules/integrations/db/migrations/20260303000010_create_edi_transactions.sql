-- Integrations Module: EDI Transaction Set Framework
--
-- Durable record for every EDI transaction (X12, EDIFACT, etc.) processed
-- by the platform.  Tracks the validation pipeline:
--   inbound:  ingested → parsed → validated → accepted | rejected
--   outbound: created  → validated → emitted  | rejected
--
-- Every status transition is Guard → Mutation → Outbox atomic.
-- Idempotency key prevents duplicate ingestion.
-- Tenant-scoped: every query filters on tenant_id.

CREATE TABLE integrations_edi_transactions (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id          TEXT    NOT NULL,
    transaction_type   TEXT    NOT NULL,   -- e.g. '850', '810', 'ORDERS', 'INVOIC'
    version            TEXT    NOT NULL,   -- e.g. '004010' (X12), 'D.96A' (EDIFACT)
    direction          TEXT    NOT NULL,   -- 'inbound' or 'outbound'
    raw_payload        TEXT,               -- original EDI content
    parsed_payload     JSONB,              -- structured representation after parsing
    validation_status  TEXT    NOT NULL DEFAULT 'ingested',
    error_details      TEXT,               -- populated on rejection
    idempotency_key    TEXT,               -- caller-supplied dedup key
    created_at         TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Direction must be known
    CONSTRAINT integrations_edi_transactions_direction_check
        CHECK (direction IN ('inbound', 'outbound')),

    -- Status must be a known pipeline value
    CONSTRAINT integrations_edi_transactions_status_check
        CHECK (validation_status IN (
            'ingested', 'created', 'parsed', 'validated',
            'accepted', 'rejected', 'emitted'
        )),

    -- One idempotency key per tenant
    CONSTRAINT integrations_edi_transactions_tenant_idem_unique
        UNIQUE (tenant_id, idempotency_key)
);

-- Tenant listing / filtering
CREATE INDEX idx_integrations_edi_transactions_tenant
    ON integrations_edi_transactions(tenant_id);

-- Find transactions by status (e.g. poll for 'ingested' to pick up)
CREATE INDEX idx_integrations_edi_transactions_status
    ON integrations_edi_transactions(tenant_id, validation_status);

-- Filter by direction
CREATE INDEX idx_integrations_edi_transactions_direction
    ON integrations_edi_transactions(tenant_id, direction);

-- Recent transactions
CREATE INDEX idx_integrations_edi_transactions_updated
    ON integrations_edi_transactions(updated_at);
