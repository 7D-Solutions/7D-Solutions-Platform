-- Accrual Reversals (Phase 24b, bd-2ob)
--
-- Tracks reversal journal entries linked back to their original accrual instance.
-- Each accrual can be reversed at most once (exactly-once guarantee).
--
-- Schema invariants:
--   1. reversal_id is a stable, deterministic UUID derived from the original accrual_id
--   2. UNIQUE on original_accrual_id prevents double reversal (exactly-once)
--   3. idempotency_key prevents duplicate reversals on replay
--   4. journal_entry_id links to the reversing GL journal entry

CREATE TABLE IF NOT EXISTS gl_accrual_reversals (
    id                      SERIAL          PRIMARY KEY,
    reversal_id             UUID            NOT NULL UNIQUE,
    original_accrual_id     UUID            NOT NULL UNIQUE,
    original_instance_id    UUID            NOT NULL REFERENCES gl_accrual_instances(instance_id) ON DELETE RESTRICT,
    tenant_id               VARCHAR(50)     NOT NULL,
    reversal_period         VARCHAR(7)      NOT NULL,  -- YYYY-MM
    reversal_date           DATE            NOT NULL,
    debit_account           VARCHAR(50)     NOT NULL,
    credit_account          VARCHAR(50)     NOT NULL,
    amount_minor            BIGINT          NOT NULL CHECK (amount_minor > 0),
    currency                VARCHAR(3)      NOT NULL,
    journal_entry_id        UUID            NOT NULL,
    outbox_event_id         UUID            NOT NULL,
    idempotency_key         VARCHAR(255)    NOT NULL,
    reason                  VARCHAR(100)    NOT NULL DEFAULT 'auto_reverse_next_period',
    created_at              TIMESTAMPTZ     NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_accrual_reversal_idempotency UNIQUE (idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_accrual_reversals_tenant
    ON gl_accrual_reversals(tenant_id);
CREATE INDEX IF NOT EXISTS idx_accrual_reversals_period
    ON gl_accrual_reversals(tenant_id, reversal_period);
CREATE INDEX IF NOT EXISTS idx_accrual_reversals_original
    ON gl_accrual_reversals(original_accrual_id);

COMMENT ON TABLE gl_accrual_reversals IS 'Exactly-once accrual reversal records (Phase 24b). Links reversal journal back to original accrual.';
