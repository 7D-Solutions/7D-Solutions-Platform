-- Asset tracking enhancements for general asset tracking (bd-127te)
-- Adds maintenance_schedule and idempotency_key to maintainable_assets.

ALTER TABLE maintainable_assets
    ADD COLUMN maintenance_schedule JSONB,
    ADD COLUMN idempotency_key TEXT;

-- Idempotency key must be unique per tenant
CREATE UNIQUE INDEX idx_maintainable_assets_tenant_idempotency
    ON maintainable_assets (tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
