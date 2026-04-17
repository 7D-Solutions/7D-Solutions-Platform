-- Outside Processing module: initial schema
-- Tables: op_orders, op_ship_events, op_return_events, op_vendor_reviews,
--         op_re_identifications, op_status_labels, op_service_type_labels, op_outbox.

-- ── OP Orders ────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS op_orders (
    op_order_id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    op_order_number     TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'draft',
    vendor_id           UUID,
    service_type        TEXT,
    service_description TEXT,
    process_spec_ref    TEXT,
    part_number         TEXT,
    part_revision       TEXT,
    quantity_sent       INTEGER NOT NULL DEFAULT 0,
    unit_of_measure     TEXT NOT NULL DEFAULT 'ea',
    work_order_id       UUID,
    operation_id        UUID,
    purchase_order_id   UUID,
    lot_id              UUID,
    serial_numbers      TEXT[] NOT NULL DEFAULT '{}',
    expected_ship_date  DATE,
    expected_return_date DATE,
    estimated_cost_cents BIGINT,
    actual_cost_cents   BIGINT,
    notes               TEXT,
    created_by          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, op_order_number)
);

CREATE INDEX IF NOT EXISTS idx_op_orders_tenant_status
    ON op_orders (tenant_id, status);

CREATE INDEX IF NOT EXISTS idx_op_orders_tenant_vendor
    ON op_orders (tenant_id, vendor_id);

CREATE INDEX IF NOT EXISTS idx_op_orders_tenant_work_order
    ON op_orders (tenant_id, work_order_id);

-- ── Ship Events ───────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS op_ship_events (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    op_order_id         UUID NOT NULL REFERENCES op_orders(op_order_id),
    ship_date           DATE NOT NULL,
    quantity_shipped    INTEGER NOT NULL,
    unit_of_measure     TEXT NOT NULL DEFAULT 'ea',
    lot_number          TEXT,
    serial_numbers      TEXT[] NOT NULL DEFAULT '{}',
    carrier_name        TEXT,
    tracking_number     TEXT,
    packing_slip_number TEXT,
    shipped_by          TEXT NOT NULL,
    shipping_reference  UUID,
    notes               TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_op_ship_events_order
    ON op_ship_events (op_order_id);

-- ── Return Events ─────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS op_return_events (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id               TEXT NOT NULL,
    op_order_id             UUID NOT NULL REFERENCES op_orders(op_order_id),
    received_date           DATE NOT NULL,
    quantity_received       INTEGER NOT NULL,
    unit_of_measure         TEXT NOT NULL DEFAULT 'ea',
    condition               TEXT NOT NULL DEFAULT 'good',
    discrepancy_notes       TEXT,
    lot_number              TEXT,
    serial_numbers          TEXT[] NOT NULL DEFAULT '{}',
    cert_ref                TEXT,
    vendor_packing_slip     TEXT,
    carrier_name            TEXT,
    tracking_number         TEXT,
    re_identification_required BOOLEAN NOT NULL DEFAULT FALSE,
    received_by             TEXT NOT NULL,
    notes                   TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_op_return_events_order
    ON op_return_events (op_order_id);

-- ── Vendor Reviews ────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS op_vendor_reviews (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    op_order_id     UUID NOT NULL REFERENCES op_orders(op_order_id),
    return_event_id UUID NOT NULL REFERENCES op_return_events(id),
    outcome         TEXT NOT NULL,
    conditions      TEXT,
    rejection_reason TEXT,
    reviewed_by     TEXT NOT NULL,
    reviewed_at     TIMESTAMPTZ NOT NULL,
    notes           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_op_vendor_reviews_order
    ON op_vendor_reviews (op_order_id);

-- ── Re-Identifications ────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS op_re_identifications (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    op_order_id     UUID NOT NULL REFERENCES op_orders(op_order_id),
    return_event_id UUID NOT NULL REFERENCES op_return_events(id),
    old_part_number TEXT NOT NULL,
    old_part_revision TEXT,
    new_part_number TEXT NOT NULL,
    new_part_revision TEXT,
    reason          TEXT NOT NULL,
    performed_by    TEXT NOT NULL,
    performed_at    TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_op_re_id_order
    ON op_re_identifications (op_order_id);

-- ── Status Labels ─────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS op_status_labels (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    canonical_status TEXT NOT NULL,
    display_label   TEXT NOT NULL,
    description     TEXT,
    updated_by      TEXT NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, canonical_status)
);

-- ── Service Type Labels ───────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS op_service_type_labels (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    service_type    TEXT NOT NULL,
    display_label   TEXT NOT NULL,
    description     TEXT,
    updated_by      TEXT NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, service_type)
);

-- ── Outbox ────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS op_outbox (
    id              BIGSERIAL PRIMARY KEY,
    event_id        UUID NOT NULL UNIQUE,
    event_type      TEXT NOT NULL,
    aggregate_type  TEXT NOT NULL,
    aggregate_id    TEXT NOT NULL,
    tenant_id       TEXT NOT NULL,
    payload         JSONB NOT NULL,
    correlation_id  TEXT,
    causation_id    TEXT,
    schema_version  TEXT NOT NULL DEFAULT '1.0.0',
    published       BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_op_outbox_unpublished
    ON op_outbox (created_at) WHERE published = FALSE;

-- ── Processed Events ─────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS op_processed_events (
    id          BIGSERIAL PRIMARY KEY,
    event_id    UUID NOT NULL UNIQUE,
    event_type  TEXT NOT NULL,
    processor   TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_op_processed_events_event_id
    ON op_processed_events (event_id);
