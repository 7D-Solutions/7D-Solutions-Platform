-- Integrations Module: File Import/Export Jobs
--
-- Durable record for every file import/export processed by the platform.
-- Tracks lifecycle (created → processing → completed/failed) with full
-- audit trail.  Idempotency key prevents duplicate job creation.
--
-- Tenant-scoped: every query filters on tenant_id.

CREATE TABLE integrations_file_jobs (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT    NOT NULL,
    file_ref         TEXT    NOT NULL,   -- storage location / upload reference
    parser_type      TEXT    NOT NULL,   -- e.g. 'csv', 'edi', 'xlsx'
    status           TEXT    NOT NULL DEFAULT 'created',
    error_details    TEXT,               -- populated when status = 'failed'
    idempotency_key  TEXT,               -- optional caller-supplied dedup key
    created_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- One idempotency key per tenant
    CONSTRAINT integrations_file_jobs_tenant_idem_unique
        UNIQUE (tenant_id, idempotency_key),

    -- Status must be a known value
    CONSTRAINT integrations_file_jobs_status_check
        CHECK (status IN ('created', 'processing', 'completed', 'failed'))
);

-- Tenant listing / filtering
CREATE INDEX idx_integrations_file_jobs_tenant
    ON integrations_file_jobs(tenant_id);

-- Find jobs by status (e.g. poll for 'created' jobs to pick up)
CREATE INDEX idx_integrations_file_jobs_status
    ON integrations_file_jobs(tenant_id, status);

-- Recent jobs
CREATE INDEX idx_integrations_file_jobs_updated
    ON integrations_file_jobs(updated_at);
