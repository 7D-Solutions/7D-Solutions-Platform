-- tt-core initial schema: TrashTech operational domain tables

CREATE TABLE IF NOT EXISTS routes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id TEXT NOT NULL,
    name TEXT NOT NULL,
    date DATE NOT NULL,
    status TEXT NOT NULL DEFAULT 'planned',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_routes_app_id_date ON routes (app_id, date);

CREATE TABLE IF NOT EXISTS pickup_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_party_id UUID NOT NULL,
    ar_customer_id INT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    scheduled_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    driver_id UUID,
    route_id UUID REFERENCES routes(id),
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_pickup_jobs_status ON pickup_jobs (status);
CREATE INDEX IF NOT EXISTS idx_pickup_jobs_route_id ON pickup_jobs (route_id);
CREATE INDEX IF NOT EXISTS idx_pickup_jobs_customer_party_id ON pickup_jobs (customer_party_id);

CREATE TABLE IF NOT EXISTS route_stops (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    route_id UUID NOT NULL REFERENCES routes(id),
    pickup_job_id UUID NOT NULL REFERENCES pickup_jobs(id),
    sequence_num INT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
);

CREATE INDEX IF NOT EXISTS idx_route_stops_route_id ON route_stops (route_id);

CREATE TABLE IF NOT EXISTS gps_pings (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    driver_id UUID NOT NULL,
    route_id UUID,
    latitude NUMERIC(10,7) NOT NULL,
    longitude NUMERIC(10,7) NOT NULL,
    accuracy_meters NUMERIC(6,2),
    recorded_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_gps_pings_driver_id ON gps_pings (driver_id);
CREATE INDEX IF NOT EXISTS idx_gps_pings_route_id ON gps_pings (route_id);

CREATE TABLE IF NOT EXISTS evidence_records (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pickup_job_id UUID NOT NULL REFERENCES pickup_jobs(id),
    evidence_type TEXT NOT NULL CHECK (evidence_type IN ('rfid_scan', 'camera_timestamp', 'driver_note')),
    payload JSONB NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_evidence_records_pickup_job_id ON evidence_records (pickup_job_id);
