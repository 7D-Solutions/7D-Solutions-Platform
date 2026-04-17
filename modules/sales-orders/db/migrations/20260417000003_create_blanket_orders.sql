-- Blanket Orders schema

CREATE TABLE blanket_orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    blanket_order_number TEXT NOT NULL,
    title TEXT NOT NULL,
    customer_id UUID,
    party_id UUID,
    status TEXT NOT NULL DEFAULT 'draft',
    currency CHAR(3) NOT NULL,
    total_committed_value_cents BIGINT NOT NULL DEFAULT 0,
    valid_from DATE,
    valid_until DATE,
    payment_terms TEXT,
    delivery_terms TEXT,
    incoterms TEXT,
    external_quote_ref TEXT,
    notes TEXT,
    created_by TEXT NOT NULL DEFAULT 'system',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT blanket_orders_status_check CHECK (
        status IN ('draft', 'active', 'expired', 'cancelled', 'closed')
    )
);

CREATE UNIQUE INDEX idx_blanket_orders_number_tenant
    ON blanket_orders (tenant_id, blanket_order_number);
CREATE INDEX idx_blanket_orders_tenant ON blanket_orders (tenant_id);
CREATE INDEX idx_blanket_orders_status ON blanket_orders (tenant_id, status);
CREATE INDEX idx_blanket_orders_valid_until ON blanket_orders (valid_until)
    WHERE status = 'active';

CREATE TABLE blanket_order_lines (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    blanket_order_id UUID NOT NULL REFERENCES blanket_orders(id) ON DELETE CASCADE,
    line_number INT NOT NULL,
    item_id UUID,
    part_number TEXT,
    part_description TEXT NOT NULL,
    uom TEXT NOT NULL DEFAULT 'EA',
    unit_price_cents BIGINT NOT NULL,
    committed_qty NUMERIC(18,4) NOT NULL,
    released_qty NUMERIC(18,4) NOT NULL DEFAULT 0,
    shipped_qty NUMERIC(18,4) NOT NULL DEFAULT 0,
    notes TEXT,
    CONSTRAINT blanket_order_lines_committed_qty_pos CHECK (committed_qty > 0),
    CONSTRAINT blanket_order_lines_released_nonneg CHECK (released_qty >= 0),
    CONSTRAINT blanket_order_lines_shipped_nonneg CHECK (shipped_qty >= 0)
);

CREATE UNIQUE INDEX idx_blanket_order_lines_number
    ON blanket_order_lines (blanket_order_id, line_number);
CREATE INDEX idx_blanket_order_lines_blanket ON blanket_order_lines (tenant_id, blanket_order_id);

CREATE TABLE blanket_order_releases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    blanket_order_id UUID NOT NULL REFERENCES blanket_orders(id),
    blanket_order_line_id UUID NOT NULL REFERENCES blanket_order_lines(id),
    release_number TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    release_qty NUMERIC(18,4) NOT NULL,
    shipped_qty NUMERIC(18,4) NOT NULL DEFAULT 0,
    requested_delivery_date DATE,
    promised_delivery_date DATE,
    actual_ship_date DATE,
    ship_to_address_id UUID,
    shipping_reference TEXT,
    sales_order_id UUID REFERENCES sales_orders(id),
    notes TEXT,
    created_by TEXT NOT NULL DEFAULT 'system',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT blanket_order_releases_status_check CHECK (
        status IN ('pending', 'released', 'shipped', 'cancelled')
    ),
    CONSTRAINT blanket_order_releases_qty_pos CHECK (release_qty > 0)
);

CREATE UNIQUE INDEX idx_blanket_order_releases_number
    ON blanket_order_releases (blanket_order_id, release_number);
CREATE INDEX idx_blanket_order_releases_blanket ON blanket_order_releases (tenant_id, blanket_order_id);
CREATE INDEX idx_blanket_order_releases_line ON blanket_order_releases (blanket_order_line_id);
