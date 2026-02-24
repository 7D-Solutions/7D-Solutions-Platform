-- Maintenance Tenant Configuration
-- Per-tenant settings controlling auto-creation and approval gate.

CREATE TABLE maintenance_tenant_config (
    tenant_id          TEXT PRIMARY KEY,
    auto_create_on_due BOOLEAN NOT NULL DEFAULT FALSE,
    approvals_required BOOLEAN NOT NULL DEFAULT FALSE,
    created_at         TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);
