-- Status label tables for per-tenant display customisation.
-- Tenants can rename canonical statuses but cannot add/remove them.

CREATE TABLE sales_order_status_labels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    canonical_status TEXT NOT NULL,
    display_label TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by TEXT NOT NULL DEFAULT 'system',
    CONSTRAINT so_status_labels_canonical_check CHECK (
        canonical_status IN ('draft', 'booked', 'in_fulfillment', 'shipped', 'closed', 'cancelled')
    ),
    UNIQUE (tenant_id, canonical_status)
);

CREATE INDEX idx_so_status_labels_tenant ON sales_order_status_labels (tenant_id);

CREATE TABLE blanket_order_status_labels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    canonical_status TEXT NOT NULL,
    display_label TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by TEXT NOT NULL DEFAULT 'system',
    CONSTRAINT bo_status_labels_canonical_check CHECK (
        canonical_status IN ('draft', 'active', 'expired', 'cancelled', 'closed')
    ),
    UNIQUE (tenant_id, canonical_status)
);

CREATE INDEX idx_bo_status_labels_tenant ON blanket_order_status_labels (tenant_id);
