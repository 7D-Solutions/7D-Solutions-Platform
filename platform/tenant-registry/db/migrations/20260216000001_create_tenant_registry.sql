-- Tenant Registry Schema
-- Central control-plane registry for tracking tenant provisioning and lifecycle
-- Provides single source of truth for tenant status, schema versions, and environment
-- PostgreSQL with SQLx for Rust backend

-- ============================================================
-- TENANT REGISTRY TABLE
-- ============================================================

CREATE TABLE tenants (
    -- Tenant identification
    tenant_id UUID PRIMARY KEY,

    -- Lifecycle status
    -- pending: record created, provisioning not yet started
    -- provisioning: databases and migrations are running
    -- active: fully provisioned and operational
    -- failed: provisioning failed (can retry)
    -- suspended: operationally suspended (data retained)
    -- deleted: soft-deleted (marked for cleanup)
    status VARCHAR(20) NOT NULL CHECK (status IN ('pending', 'provisioning', 'active', 'failed', 'suspended', 'deleted')),
    environment VARCHAR(20) NOT NULL CHECK (environment IN ('development', 'staging', 'production')),

    -- Module schema version tracking
    -- Stores per-module schema versions as JSON: {"ar": "20260216000001", "payments": "20260215000002", ...}
    module_schema_versions JSONB NOT NULL DEFAULT '{}'::jsonb,

    -- Audit timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Soft delete support (when status='deleted', this tracks when)
    deleted_at TIMESTAMPTZ
);

-- ============================================================
-- PROVISIONING STEPS TABLE
-- ============================================================

CREATE TABLE provisioning_steps (
    -- Step identification
    step_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,

    -- Step definition
    step_name VARCHAR(100) NOT NULL,
    step_order INTEGER NOT NULL,

    -- Step execution tracking
    status VARCHAR(20) NOT NULL CHECK (status IN ('pending', 'in_progress', 'completed', 'failed')),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,

    -- Error tracking for failed steps
    error_message TEXT,

    -- Verification check (JSON representation of what was verified)
    verification_result JSONB,

    -- Audit timestamp
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Ensure each step appears once per tenant
    UNIQUE (tenant_id, step_name)
);

-- ============================================================
-- INDEXES FOR QUERY PERFORMANCE
-- ============================================================

-- Index for finding tenants by status
CREATE INDEX tenants_status ON tenants(status);

-- Index for finding tenants by environment
CREATE INDEX tenants_environment ON tenants(environment);

-- Index for finding recently created tenants
CREATE INDEX tenants_created_at ON tenants(created_at DESC);

-- Index for finding all provisioning steps for a tenant (ordered by step_order)
CREATE INDEX provisioning_steps_tenant_id ON provisioning_steps(tenant_id, step_order);

-- Index for finding failed provisioning steps
CREATE INDEX provisioning_steps_failed ON provisioning_steps(status) WHERE status = 'failed';

-- ============================================================
-- COMMENTS
-- ============================================================

COMMENT ON TABLE tenants IS 'Central registry for all tenants across the platform';
COMMENT ON COLUMN tenants.tenant_id IS 'Globally unique tenant identifier';
COMMENT ON COLUMN tenants.status IS 'Current lifecycle status of the tenant';
COMMENT ON COLUMN tenants.environment IS 'Deployment environment (development, staging, production)';
COMMENT ON COLUMN tenants.module_schema_versions IS 'Per-module schema versions for upgrade tracking';
COMMENT ON COLUMN tenants.created_at IS 'Timestamp when tenant was first created';
COMMENT ON COLUMN tenants.updated_at IS 'Timestamp when tenant record was last modified';
COMMENT ON COLUMN tenants.deleted_at IS 'Timestamp when tenant was soft-deleted (if status=deleted)';

COMMENT ON TABLE provisioning_steps IS 'Tracks deterministic provisioning steps for each tenant';
COMMENT ON COLUMN provisioning_steps.step_name IS 'Name of provisioning step (e.g., "create_databases", "run_migrations")';
COMMENT ON COLUMN provisioning_steps.step_order IS 'Order in which this step should be executed';
COMMENT ON COLUMN provisioning_steps.status IS 'Current execution status of this provisioning step';
COMMENT ON COLUMN provisioning_steps.verification_result IS 'JSON representation of verification check results';
