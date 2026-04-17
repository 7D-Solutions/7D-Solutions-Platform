-- Direct cost accumulation for work orders.
-- Posting categories: labor, material, outside_processing, scrap, other, overhead
-- Invariant: work_order_cost_summaries.total_cost_cents = SUM(work_order_cost_postings.amount_cents)
--            for all postings with the same (work_order_id, tenant_id).
--            Enforced by post_cost doing both writes in a single transaction.

CREATE TABLE IF NOT EXISTS work_order_cost_postings (
    posting_id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    work_order_id       UUID NOT NULL REFERENCES work_orders(work_order_id),
    operation_id        UUID REFERENCES operations(operation_id),
    posting_category    TEXT NOT NULL CHECK (posting_category IN (
                            'labor', 'material', 'outside_processing',
                            'scrap', 'overhead', 'other'
                        )),
    amount_cents        BIGINT NOT NULL,
    quantity            FLOAT8,
    source_event_id     UUID,
    posted_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    posted_by           TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cost_postings_wo
    ON work_order_cost_postings (work_order_id, tenant_id);

CREATE INDEX IF NOT EXISTS idx_cost_postings_tenant
    ON work_order_cost_postings (tenant_id);

-- Unique on source_event_id to make event consumer idempotent.
-- NULLs are excluded from unique constraints in Postgres, so manual postings
-- (source_event_id = NULL) are always allowed.
CREATE UNIQUE INDEX IF NOT EXISTS idx_cost_postings_source_event_idempotency
    ON work_order_cost_postings (source_event_id)
    WHERE source_event_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS work_order_cost_summaries (
    work_order_id           UUID NOT NULL,
    tenant_id               TEXT NOT NULL,
    total_cost_cents        BIGINT NOT NULL DEFAULT 0,
    labor_cost_cents        BIGINT NOT NULL DEFAULT 0,
    material_cost_cents     BIGINT NOT NULL DEFAULT 0,
    osp_cost_cents          BIGINT NOT NULL DEFAULT 0,
    scrap_cost_cents        BIGINT NOT NULL DEFAULT 0,
    overhead_cost_cents     BIGINT NOT NULL DEFAULT 0,
    other_cost_cents        BIGINT NOT NULL DEFAULT 0,
    posting_count           INT NOT NULL DEFAULT 0,
    last_updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (work_order_id, tenant_id)
);

CREATE INDEX IF NOT EXISTS idx_cost_summaries_tenant
    ON work_order_cost_summaries (tenant_id);
