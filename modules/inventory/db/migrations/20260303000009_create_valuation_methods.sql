-- Inventory: Multi-Method Valuation Support
--
-- Adds LIFO, WAC (weighted average cost), and standard cost valuation
-- methods alongside the existing FIFO snapshot system.
--
-- item_valuation_configs:
--   Per-item valuation method selection with tenant scoping.
--   Defaults to FIFO when no config exists for an item.
--
-- valuation_runs:
--   Each row is one point-in-time valuation execution for a given
--   method, tenant, and warehouse. Idempotent via inv_idempotency_keys.
--
-- valuation_run_lines:
--   Per-item detail under a valuation run. Includes variance for
--   standard cost method.

-- ─── Item valuation method configuration ──────────────────────────────────────

CREATE TABLE item_valuation_configs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    item_id             UUID NOT NULL REFERENCES items(id),
    method              TEXT NOT NULL CHECK (method IN ('fifo', 'lifo', 'wac', 'standard_cost')),
    -- Standard cost in minor currency units; required when method = 'standard_cost'
    standard_cost_minor BIGINT CHECK (
        (method != 'standard_cost') OR (standard_cost_minor IS NOT NULL AND standard_cost_minor >= 0)
    ),
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- One valuation method per item per tenant
CREATE UNIQUE INDEX idx_val_config_tenant_item
    ON item_valuation_configs(tenant_id, item_id);

CREATE INDEX idx_val_config_tenant
    ON item_valuation_configs(tenant_id);

-- ─── Valuation run header ─────────────────────────────────────────────────────

CREATE TABLE valuation_runs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    warehouse_id        UUID NOT NULL,
    method              TEXT NOT NULL CHECK (method IN ('fifo', 'lifo', 'wac', 'standard_cost')),
    as_of               TIMESTAMP WITH TIME ZONE NOT NULL,
    total_value_minor   BIGINT NOT NULL CHECK (total_value_minor >= 0),
    total_cogs_minor    BIGINT NOT NULL DEFAULT 0,
    currency            TEXT NOT NULL DEFAULT 'usd',
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_val_runs_tenant_wh
    ON valuation_runs(tenant_id, warehouse_id, as_of DESC);

-- ─── Per-item valuation run lines ─────────────────────────────────────────────

CREATE TABLE valuation_run_lines (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id              UUID NOT NULL REFERENCES valuation_runs(id) ON DELETE CASCADE,
    item_id             UUID NOT NULL REFERENCES items(id),
    warehouse_id        UUID NOT NULL,
    quantity_on_hand    BIGINT NOT NULL CHECK (quantity_on_hand >= 0),
    unit_cost_minor     BIGINT NOT NULL CHECK (unit_cost_minor >= 0),
    total_value_minor   BIGINT NOT NULL CHECK (total_value_minor >= 0),
    -- Variance from standard cost (actual - standard) * qty; 0 for non-standard methods
    variance_minor      BIGINT NOT NULL DEFAULT 0,
    currency            TEXT NOT NULL DEFAULT 'usd'
);

CREATE INDEX idx_val_run_lines_run
    ON valuation_run_lines(run_id);

CREATE INDEX idx_val_run_lines_item
    ON valuation_run_lines(item_id, run_id);

-- One line per (item, warehouse) per run
CREATE UNIQUE INDEX val_run_lines_unique
    ON valuation_run_lines(run_id, item_id, warehouse_id);
