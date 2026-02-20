-- TTP metering: per-event usage tracking + pricing rules
--
-- ttp_metering_events: raw usage events with idempotent ingestion
-- ttp_metering_pricing: per-dimension pricing rules for trace computation

-- Metering events: raw usage data points
CREATE TABLE IF NOT EXISTS ttp_metering_events (
    event_id        UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID        NOT NULL,
    dimension       TEXT        NOT NULL,   -- e.g. 'api_calls', 'storage_gb'
    quantity        BIGINT      NOT NULL CHECK (quantity > 0),
    occurred_at     TIMESTAMPTZ NOT NULL,
    idempotency_key TEXT        NOT NULL,
    source_ref      TEXT,                   -- optional external reference
    ingested_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_ttp_metering_idempotency
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_ttp_metering_events_tenant_period
    ON ttp_metering_events (tenant_id, occurred_at);

CREATE INDEX IF NOT EXISTS idx_ttp_metering_events_tenant_dimension
    ON ttp_metering_events (tenant_id, dimension, occurred_at);

-- Metering pricing rules: unit price per dimension
-- Effective date ranges allow price changes without losing history.
CREATE TABLE IF NOT EXISTS ttp_metering_pricing (
    pricing_id       UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        UUID        NOT NULL,
    dimension        TEXT        NOT NULL,
    unit_price_minor BIGINT      NOT NULL CHECK (unit_price_minor >= 0),
    currency         CHAR(3)     NOT NULL,
    effective_from   DATE        NOT NULL,
    effective_to     DATE,       -- NULL = currently active
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_ttp_metering_pricing_dimension_from
        UNIQUE (tenant_id, dimension, effective_from)
);

CREATE INDEX IF NOT EXISTS idx_ttp_metering_pricing_lookup
    ON ttp_metering_pricing (tenant_id, dimension, effective_from);
