-- FX Rate Store (Phase 23a, bd-104)
--
-- Append-only table of exchange rate snapshots.
-- Rates are never updated or deleted — each insert is a new snapshot.
-- Idempotency is enforced by a UNIQUE constraint on idempotency_key.

CREATE TABLE IF NOT EXISTS fx_rates (
    id              UUID PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    base_currency   TEXT NOT NULL,        -- ISO 4217 (e.g. "EUR")
    quote_currency  TEXT NOT NULL,        -- ISO 4217 (e.g. "USD")
    rate            DOUBLE PRECISION NOT NULL,  -- 1 base = rate quote
    inverse_rate    DOUBLE PRECISION NOT NULL,  -- 1 quote = inverse_rate base
    effective_at    TIMESTAMPTZ NOT NULL, -- when this rate becomes active
    source          TEXT NOT NULL,        -- provider id (e.g. "ecb", "manual")
    idempotency_key TEXT NOT NULL,        -- caller-supplied dedup key
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Idempotency: duplicate inserts with same key are silently skipped
CREATE UNIQUE INDEX IF NOT EXISTS idx_fx_rates_idempotency
    ON fx_rates (idempotency_key);

-- Latest-as-of query: find the most recent rate for a pair at or before a timestamp
-- Covers: WHERE tenant_id = $1 AND base_currency = $2 AND quote_currency = $3 AND effective_at <= $4
--         ORDER BY effective_at DESC LIMIT 1
CREATE INDEX IF NOT EXISTS idx_fx_rates_latest_lookup
    ON fx_rates (tenant_id, base_currency, quote_currency, effective_at DESC);
