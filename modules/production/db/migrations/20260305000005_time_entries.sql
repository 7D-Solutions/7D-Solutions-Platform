-- Timekeeping: append-only time entries linked to work orders and optional operations.

CREATE TABLE IF NOT EXISTS time_entries (
    time_entry_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    work_order_id       UUID NOT NULL REFERENCES work_orders(work_order_id),
    operation_id        UUID REFERENCES operations(operation_id),
    actor_id            TEXT NOT NULL,
    start_ts            TIMESTAMPTZ NOT NULL,
    end_ts              TIMESTAMPTZ,
    minutes             INTEGER,
    notes               TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_time_entries_work_order
    ON time_entries (work_order_id);

CREATE INDEX IF NOT EXISTS idx_time_entries_tenant
    ON time_entries (tenant_id);

CREATE INDEX IF NOT EXISTS idx_time_entries_actor
    ON time_entries (tenant_id, actor_id);
