-- Meter Readings
-- Timestamped readings per asset per meter type.
-- Monotonicity enforced in application code (rollover detection needs context).
-- Validation is against highest reading_value, not latest timestamp.

CREATE TABLE meter_readings (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id     TEXT NOT NULL,
    asset_id      UUID NOT NULL REFERENCES maintainable_assets(id),
    meter_type_id UUID NOT NULL REFERENCES meter_types(id),
    reading_value BIGINT NOT NULL,
    recorded_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    recorded_by   TEXT,
    created_at    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT meter_readings_value_non_negative CHECK (reading_value >= 0)
);

CREATE INDEX idx_meter_readings_tenant ON meter_readings(tenant_id);
CREATE INDEX idx_meter_readings_lookup ON meter_readings(tenant_id, asset_id, meter_type_id, recorded_at);
