-- Sales Orders schema
-- All monetary values are integer cents per platform standard (AR-MODULE-SPEC pattern).

CREATE TABLE sales_orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    order_number TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft',
    customer_id UUID,
    party_id UUID,
    currency CHAR(3) NOT NULL,
    subtotal_cents BIGINT NOT NULL DEFAULT 0,
    tax_cents BIGINT NOT NULL DEFAULT 0,
    total_cents BIGINT NOT NULL DEFAULT 0,
    order_date DATE NOT NULL DEFAULT CURRENT_DATE,
    required_date DATE,
    promised_date DATE,
    external_quote_ref TEXT,
    blanket_order_id UUID,
    blanket_release_id UUID,
    notes TEXT,
    created_by TEXT NOT NULL DEFAULT 'system',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT sales_orders_status_check CHECK (
        status IN ('draft', 'booked', 'in_fulfillment', 'shipped', 'closed', 'cancelled')
    )
);

CREATE UNIQUE INDEX idx_sales_orders_order_number_tenant
    ON sales_orders (tenant_id, order_number);
CREATE INDEX idx_sales_orders_tenant ON sales_orders (tenant_id);
CREATE INDEX idx_sales_orders_customer ON sales_orders (tenant_id, customer_id);
CREATE INDEX idx_sales_orders_status ON sales_orders (tenant_id, status);
CREATE INDEX idx_sales_orders_blanket ON sales_orders (tenant_id, blanket_order_id)
    WHERE blanket_order_id IS NOT NULL;

CREATE TABLE sales_order_lines (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    sales_order_id UUID NOT NULL REFERENCES sales_orders(id) ON DELETE CASCADE,
    line_number INT NOT NULL,
    item_id UUID,
    part_number TEXT,
    description TEXT NOT NULL,
    uom TEXT NOT NULL DEFAULT 'EA',
    quantity NUMERIC(18,4) NOT NULL,
    unit_price_cents BIGINT NOT NULL,
    line_total_cents BIGINT NOT NULL,
    required_date DATE,
    promised_date DATE,
    shipped_qty NUMERIC(18,4) NOT NULL DEFAULT 0,
    warehouse_id UUID,
    reservation_id UUID,
    invoiced_at TIMESTAMPTZ,
    notes TEXT,
    CONSTRAINT sales_order_lines_qty_positive CHECK (quantity > 0),
    CONSTRAINT sales_order_lines_unit_price_nonneg CHECK (unit_price_cents >= 0)
);

CREATE UNIQUE INDEX idx_sales_order_lines_line_number
    ON sales_order_lines (sales_order_id, line_number);
CREATE INDEX idx_sales_order_lines_order ON sales_order_lines (tenant_id, sales_order_id);
CREATE INDEX idx_sales_order_lines_item ON sales_order_lines (tenant_id, item_id)
    WHERE item_id IS NOT NULL;
