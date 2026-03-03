-- Inventory lot expiry and alert dedupe state (bd-3m9ss).
--
-- Expiry is modeled directly on inventory_lots for deterministic traceability.
-- Alert dedupe state ensures replay-safe scans that do not emit duplicate alerts.

ALTER TABLE inventory_lots
    ADD COLUMN expires_on DATE,
    ADD COLUMN expiry_source TEXT,
    ADD COLUMN expiry_set_at TIMESTAMP WITH TIME ZONE;

ALTER TABLE inventory_lots
    ADD CONSTRAINT inventory_lots_expiry_source_check_v2
    CHECK (expiry_source IS NULL OR expiry_source IN ('manual', 'policy'));

CREATE INDEX idx_lots_tenant_expiry
    ON inventory_lots (tenant_id, expires_on)
    WHERE expires_on IS NOT NULL;

CREATE TABLE inv_lot_expiry_alert_state (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    lot_id           UUID NOT NULL REFERENCES inventory_lots(id) ON DELETE CASCADE,
    alert_type       TEXT NOT NULL,             -- 'expiring_soon' | 'expired'
    alert_date       DATE NOT NULL,             -- date for which the alert was emitted
    window_days      INT NOT NULL DEFAULT 0,    -- 0 for expired alerts
    created_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT inv_lot_expiry_alert_type_check
        CHECK (alert_type IN ('expiring_soon', 'expired')),

    CONSTRAINT inv_lot_expiry_alert_window_nonnegative
        CHECK (window_days >= 0),

    CONSTRAINT inv_lot_expiry_alert_unique
        UNIQUE (tenant_id, lot_id, alert_type, alert_date, window_days)
);

CREATE INDEX idx_inv_lot_expiry_alert_tenant_date
    ON inv_lot_expiry_alert_state (tenant_id, alert_date);
