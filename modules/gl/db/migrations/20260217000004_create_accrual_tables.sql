-- Accrual Templates & Instances (Phase 24b, bd-3qa)
--
-- Templates define recurring accrual patterns (accounts, amount, reversal policy).
-- Instances are created from templates per accounting period — each instance
-- creates a balanced journal entry atomically.
--
-- Schema invariants:
--   1. template_id and instance_id are stable business keys (UUID, idempotency anchors)
--   2. Instances are append-only: no UPDATEs on financial fields post-creation
--   3. idempotency_key on instances prevents duplicate postings on retry
--   4. journal_entry_id links back to the GL journal (proof of posting)

-- ============================================================================
-- Accrual Templates
-- ============================================================================

CREATE TABLE IF NOT EXISTS gl_accrual_templates (
    id              SERIAL          PRIMARY KEY,
    template_id     UUID            NOT NULL UNIQUE,
    tenant_id       VARCHAR(50)     NOT NULL,
    name            VARCHAR(255)    NOT NULL,
    description     TEXT,
    debit_account   VARCHAR(50)     NOT NULL,
    credit_account  VARCHAR(50)     NOT NULL,
    amount_minor    BIGINT          NOT NULL CHECK (amount_minor > 0),
    currency        VARCHAR(3)      NOT NULL,
    reversal_policy JSONB           NOT NULL DEFAULT '{"auto_reverse_next_period": true}',
    cashflow_class  VARCHAR(30)     NOT NULL DEFAULT 'operating',
    active          BOOLEAN         NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_accrual_templates_tenant
    ON gl_accrual_templates(tenant_id);
CREATE INDEX IF NOT EXISTS idx_accrual_templates_active
    ON gl_accrual_templates(tenant_id, active) WHERE active = TRUE;

-- ============================================================================
-- Accrual Instances (append-only)
-- ============================================================================

CREATE TABLE IF NOT EXISTS gl_accrual_instances (
    id                  SERIAL          PRIMARY KEY,
    instance_id         UUID            NOT NULL UNIQUE,
    template_id         UUID            NOT NULL REFERENCES gl_accrual_templates(template_id) ON DELETE RESTRICT,
    tenant_id           VARCHAR(50)     NOT NULL,
    accrual_id          UUID            NOT NULL UNIQUE,
    period              VARCHAR(7)      NOT NULL,  -- YYYY-MM
    posting_date        DATE            NOT NULL,
    name                VARCHAR(255)    NOT NULL,
    debit_account       VARCHAR(50)     NOT NULL,
    credit_account      VARCHAR(50)     NOT NULL,
    amount_minor        BIGINT          NOT NULL CHECK (amount_minor > 0),
    currency            VARCHAR(3)      NOT NULL,
    reversal_policy     JSONB           NOT NULL,
    cashflow_class      VARCHAR(30)     NOT NULL,
    journal_entry_id    UUID,
    status              VARCHAR(20)     NOT NULL DEFAULT 'posted',
    idempotency_key     VARCHAR(255)    NOT NULL,
    outbox_event_id     UUID,
    created_at          TIMESTAMPTZ     NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_accrual_instance_idempotency UNIQUE (idempotency_key),
    CONSTRAINT uq_accrual_template_period UNIQUE (template_id, period)
);

CREATE INDEX IF NOT EXISTS idx_accrual_instances_tenant
    ON gl_accrual_instances(tenant_id);
CREATE INDEX IF NOT EXISTS idx_accrual_instances_period
    ON gl_accrual_instances(tenant_id, period);
CREATE INDEX IF NOT EXISTS idx_accrual_instances_template
    ON gl_accrual_instances(template_id);
CREATE INDEX IF NOT EXISTS idx_accrual_instances_status
    ON gl_accrual_instances(status);

COMMENT ON TABLE gl_accrual_templates IS 'Reusable accrual patterns (Phase 24b). Defines accounts, amount, reversal policy.';
COMMENT ON TABLE gl_accrual_instances IS 'Append-only accrual instances created from templates per period. Each instance posts a balanced GL journal entry.';
