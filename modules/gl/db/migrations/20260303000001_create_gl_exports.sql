-- GL Export tracking table
-- Tracks export requests with idempotency and tenant isolation

CREATE TABLE gl_exports (
    id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    format TEXT NOT NULL CHECK (format IN ('quickbooks', 'xero')),
    export_type TEXT NOT NULL CHECK (export_type IN ('chart_of_accounts', 'journal_entries')),
    status TEXT NOT NULL DEFAULT 'completed' CHECK (status IN ('completed', 'failed')),
    output TEXT,
    period_id UUID,
    error_message TEXT,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMP WITH TIME ZONE,
    CONSTRAINT uq_gl_exports_tenant_idempotency UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_gl_exports_tenant ON gl_exports(tenant_id);
CREATE INDEX idx_gl_exports_tenant_key ON gl_exports(tenant_id, idempotency_key);
