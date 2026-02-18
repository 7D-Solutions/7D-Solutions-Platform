-- AP Purchase Orders, Lines, and Status Log
--
-- purchase_orders: Header record for each PO sent to a vendor.
-- po_lines:        Individual line items on a PO.
-- po_status:       Append-only audit trail of PO lifecycle transitions.
--
-- All monetary fields are BIGINT (i64 minor currency units, e.g. cents).
-- Currency is ISO 4217 (e.g. "USD").

-- =============================================================================
-- purchase_orders
-- =============================================================================

CREATE TABLE purchase_orders (
    po_id                   UUID PRIMARY KEY,
    tenant_id               TEXT NOT NULL,
    vendor_id               UUID NOT NULL REFERENCES vendors (vendor_id),
    -- Human-readable PO number; unique per tenant
    po_number               TEXT NOT NULL,
    -- ISO 4217
    currency                CHAR(3) NOT NULL,
    -- Total PO value in minor currency units (i64)
    total_minor             BIGINT NOT NULL CHECK (total_minor >= 0),
    -- Current denormalized status (source of truth is po_status log)
    status                  TEXT NOT NULL DEFAULT 'draft'
                                CHECK (status IN ('draft', 'approved', 'closed', 'cancelled')),
    created_by              TEXT NOT NULL,
    created_at              TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    expected_delivery_date  TIMESTAMP WITH TIME ZONE,

    CONSTRAINT uq_po_tenant_number UNIQUE (tenant_id, po_number)
);

CREATE INDEX idx_po_tenant_vendor ON purchase_orders (tenant_id, vendor_id);
CREATE INDEX idx_po_tenant_status ON purchase_orders (tenant_id, status);

-- =============================================================================
-- po_lines
-- =============================================================================

CREATE TABLE po_lines (
    line_id             UUID PRIMARY KEY,
    po_id               UUID NOT NULL REFERENCES purchase_orders (po_id),
    description         TEXT NOT NULL,
    quantity            NUMERIC(18, 6) NOT NULL CHECK (quantity > 0),
    unit_of_measure     TEXT NOT NULL,
    -- Unit price in minor currency units (i64)
    unit_price_minor    BIGINT NOT NULL CHECK (unit_price_minor >= 0),
    -- Derived: quantity * unit_price_minor (stored for audit, not enforced)
    line_total_minor    BIGINT NOT NULL CHECK (line_total_minor >= 0),
    gl_account_code     TEXT NOT NULL,
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_po_lines_po_id ON po_lines (po_id);

-- =============================================================================
-- po_status (append-only status audit log)
-- =============================================================================

CREATE TABLE po_status (
    id          BIGSERIAL PRIMARY KEY,
    po_id       UUID NOT NULL REFERENCES purchase_orders (po_id),
    status      TEXT NOT NULL
                    CHECK (status IN ('draft', 'approved', 'closed', 'cancelled')),
    changed_by  TEXT NOT NULL,
    changed_at  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    -- Optional reason for the transition (e.g. cancellation reason)
    reason      TEXT
);

CREATE INDEX idx_po_status_po_id    ON po_status (po_id);
CREATE INDEX idx_po_status_changed  ON po_status (changed_at);
