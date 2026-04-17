-- Corrective migration: align column names and types with Rust domain structs.
-- Initial migrations used NUMERIC(18,4) for qty fields (incompatible with sqlx f64 binding)
-- and mismatched column names vs domain struct field names.

-- Drop in dependency order
DROP TABLE IF EXISTS blanket_order_releases CASCADE;
DROP TABLE IF EXISTS blanket_order_lines CASCADE;
DROP TABLE IF EXISTS blanket_orders CASCADE;
DROP TABLE IF EXISTS sales_order_lines CASCADE;

-- blanket_orders: correct column names and add released_cents
CREATE TABLE blanket_orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    blanket_number TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft',
    customer_id UUID,
    party_id UUID,
    currency CHAR(3) NOT NULL,
    committed_cents BIGINT NOT NULL DEFAULT 0,
    released_cents BIGINT NOT NULL DEFAULT 0,
    effective_date DATE NOT NULL DEFAULT CURRENT_DATE,
    expiry_date DATE,
    notes TEXT,
    created_by TEXT NOT NULL DEFAULT 'system',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT blanket_orders_status_check CHECK (
        status IN ('draft', 'active', 'expired', 'cancelled', 'closed')
    )
);

CREATE UNIQUE INDEX idx_blanket_orders_number_tenant
    ON blanket_orders (tenant_id, blanket_number);
CREATE INDEX idx_blanket_orders_tenant ON blanket_orders (tenant_id);
CREATE INDEX idx_blanket_orders_status ON blanket_orders (tenant_id, status);

-- blanket_order_lines: description (not part_description), DOUBLE PRECISION for qty
CREATE TABLE blanket_order_lines (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    blanket_order_id UUID NOT NULL REFERENCES blanket_orders(id) ON DELETE CASCADE,
    line_number INT NOT NULL,
    item_id UUID,
    part_number TEXT,
    description TEXT NOT NULL,
    uom TEXT NOT NULL DEFAULT 'EA',
    unit_price_cents BIGINT NOT NULL,
    committed_qty DOUBLE PRECISION NOT NULL,
    released_qty DOUBLE PRECISION NOT NULL DEFAULT 0,
    notes TEXT,
    CONSTRAINT blanket_order_lines_committed_qty_pos CHECK (committed_qty > 0),
    CONSTRAINT blanket_order_lines_released_nonneg CHECK (released_qty >= 0)
);

CREATE UNIQUE INDEX idx_blanket_order_lines_number
    ON blanket_order_lines (blanket_order_id, line_number);
CREATE INDEX idx_blanket_order_lines_blanket
    ON blanket_order_lines (tenant_id, blanket_order_id);

-- sales_order_lines: DOUBLE PRECISION for quantity and shipped_qty
CREATE TABLE sales_order_lines (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    sales_order_id UUID NOT NULL REFERENCES sales_orders(id) ON DELETE CASCADE,
    line_number INT NOT NULL,
    item_id UUID,
    part_number TEXT,
    description TEXT NOT NULL,
    uom TEXT NOT NULL DEFAULT 'EA',
    quantity DOUBLE PRECISION NOT NULL,
    unit_price_cents BIGINT NOT NULL,
    line_total_cents BIGINT NOT NULL,
    required_date DATE,
    promised_date DATE,
    shipped_qty DOUBLE PRECISION NOT NULL DEFAULT 0,
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

-- blanket_order_releases: blanket_line_id (not blanket_order_line_id),
--   release_date NOT NULL, DOUBLE PRECISION for qty
CREATE TABLE blanket_order_releases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    blanket_order_id UUID NOT NULL REFERENCES blanket_orders(id),
    blanket_line_id UUID NOT NULL REFERENCES blanket_order_lines(id),
    sales_order_id UUID REFERENCES sales_orders(id),
    status TEXT NOT NULL DEFAULT 'pending',
    release_qty DOUBLE PRECISION NOT NULL,
    release_date DATE NOT NULL DEFAULT CURRENT_DATE,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT blanket_order_releases_status_check CHECK (
        status IN ('pending', 'released', 'shipped', 'cancelled')
    ),
    CONSTRAINT blanket_order_releases_qty_pos CHECK (release_qty > 0)
);

CREATE INDEX idx_blanket_order_releases_blanket
    ON blanket_order_releases (tenant_id, blanket_order_id);
CREATE INDEX idx_blanket_order_releases_line ON blanket_order_releases (blanket_line_id);
