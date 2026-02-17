-- Tax Quote Cache (Phase 23b, bd-29j)
--
-- Cached provider responses keyed by (app_id, invoice_id, idempotency_key).
-- Ensures deterministic invoice totals on replay: same request_hash → same tax.
-- Provider may only be called once per invoice; subsequent reads use cache.

CREATE TABLE IF NOT EXISTS ar_tax_quote_cache (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id VARCHAR(50) NOT NULL,
    invoice_id VARCHAR(255) NOT NULL,
    idempotency_key VARCHAR(255) NOT NULL,
    request_hash VARCHAR(64) NOT NULL,
    provider VARCHAR(50) NOT NULL,
    provider_quote_ref VARCHAR(255) NOT NULL,
    total_tax_minor BIGINT NOT NULL,
    tax_by_line JSONB NOT NULL,
    response_json JSONB NOT NULL,
    quoted_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_tax_quote_cache_key UNIQUE (app_id, invoice_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_tax_quote_cache_app_invoice
    ON ar_tax_quote_cache(app_id, invoice_id);

CREATE INDEX IF NOT EXISTS idx_tax_quote_cache_hash_lookup
    ON ar_tax_quote_cache(app_id, invoice_id, request_hash);
