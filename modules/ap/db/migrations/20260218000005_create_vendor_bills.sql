-- AP Vendor Bills and Bill Lines
--
-- vendor_bills: The AP liability record for each vendor invoice.
-- bill_lines:   Individual line items on a vendor bill.
--
-- All monetary fields are BIGINT (i64 minor currency units, e.g. cents).
-- Currency is ISO 4217.
--
-- Indexed for:
--   - Open bills by due_date for payment scheduling
--   - Aging bucket queries (tenant_id, status, due_date)
--   - Vendor-scoped bill history

-- =============================================================================
-- vendor_bills
-- =============================================================================

CREATE TABLE vendor_bills (
    bill_id             UUID PRIMARY KEY,
    tenant_id           TEXT NOT NULL,
    vendor_id           UUID NOT NULL REFERENCES vendors (vendor_id),
    -- Vendor's external invoice reference (used for idempotent bill entry)
    vendor_invoice_ref  TEXT NOT NULL,
    -- ISO 4217
    currency            CHAR(3) NOT NULL,
    -- Total bill amount in minor currency units (i64)
    total_minor         BIGINT NOT NULL CHECK (total_minor >= 0),
    -- Tax amount in minor currency units (optional; i64)
    tax_minor           BIGINT CHECK (tax_minor >= 0),
    invoice_date        TIMESTAMP WITH TIME ZONE NOT NULL,
    due_date            TIMESTAMP WITH TIME ZONE NOT NULL,
    -- Bill lifecycle status
    status              TEXT NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending', 'matched', 'approved', 'paid', 'voided')),
    entered_by          TEXT NOT NULL,
    entered_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Prevent duplicate bills for the same vendor invoice per tenant
    CONSTRAINT uq_vendor_bill_invoice UNIQUE (tenant_id, vendor_id, vendor_invoice_ref)
);

-- Open bills ordered by due_date — primary payment scheduling index
CREATE INDEX idx_vendor_bills_open_due
    ON vendor_bills (tenant_id, due_date)
    WHERE status NOT IN ('paid', 'voided');

-- Aging bucket queries: (tenant_id, status, due_date)
CREATE INDEX idx_vendor_bills_aging
    ON vendor_bills (tenant_id, status, due_date);

-- Vendor-scoped bill history
CREATE INDEX idx_vendor_bills_vendor
    ON vendor_bills (tenant_id, vendor_id);

-- =============================================================================
-- bill_lines
-- =============================================================================

CREATE TABLE bill_lines (
    line_id             UUID PRIMARY KEY,
    bill_id             UUID NOT NULL REFERENCES vendor_bills (bill_id),
    description         TEXT NOT NULL,
    quantity            NUMERIC(18, 6) NOT NULL CHECK (quantity > 0),
    -- Unit price in minor currency units (i64)
    unit_price_minor    BIGINT NOT NULL CHECK (unit_price_minor >= 0),
    -- Line total in minor currency units (i64): quantity * unit_price_minor
    line_total_minor    BIGINT NOT NULL CHECK (line_total_minor >= 0),
    gl_account_code     TEXT NOT NULL,
    -- PO line this bill line references (NULL for non-PO bills)
    po_line_id          UUID REFERENCES po_lines (line_id),
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_bill_lines_bill_id    ON bill_lines (bill_id);
CREATE INDEX idx_bill_lines_po_line_id ON bill_lines (po_line_id)
    WHERE po_line_id IS NOT NULL;
