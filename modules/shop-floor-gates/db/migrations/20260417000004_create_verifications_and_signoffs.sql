-- Operation Start Verifications
-- status: pending/verified/skipped

CREATE TABLE operation_start_verifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    work_order_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    drawing_verified BOOLEAN NOT NULL DEFAULT FALSE,
    material_verified BOOLEAN NOT NULL DEFAULT FALSE,
    instruction_verified BOOLEAN NOT NULL DEFAULT FALSE,
    operator_id UUID NOT NULL,
    operator_confirmed_at TIMESTAMPTZ,
    verifier_id UUID,
    verified_at TIMESTAMPTZ,
    skipped_by UUID,
    skipped_at TIMESTAMPTZ,
    skip_reason TEXT,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT operation_start_verifications_status_check CHECK (status IN ('pending', 'verified', 'skipped')),
    CONSTRAINT operation_start_verifications_uq UNIQUE (tenant_id, work_order_id, operation_id)
);

CREATE INDEX idx_osv_tenant ON operation_start_verifications (tenant_id);
CREATE INDEX idx_osv_wo ON operation_start_verifications (tenant_id, work_order_id);
CREATE INDEX idx_osv_op ON operation_start_verifications (tenant_id, operation_id);
CREATE INDEX idx_osv_status ON operation_start_verifications (tenant_id, status);

CREATE TABLE verification_status_labels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    status_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    color_hex TEXT,
    sort_order INT NOT NULL DEFAULT 0,
    CONSTRAINT verification_status_labels_uq UNIQUE (tenant_id, status_key)
);

-- Signoffs (append-only)
-- entity_type whitelist enforced in application layer
CREATE TABLE signoffs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id UUID NOT NULL,
    role TEXT NOT NULL,
    signoff_number TEXT NOT NULL,
    signed_by UUID NOT NULL,
    signed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    signature_text TEXT NOT NULL,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_signoffs_number_tenant ON signoffs (tenant_id, signoff_number);
CREATE INDEX idx_signoffs_tenant ON signoffs (tenant_id);
CREATE INDEX idx_signoffs_entity ON signoffs (tenant_id, entity_type, entity_id);
CREATE INDEX idx_signoffs_role ON signoffs (tenant_id, role);

CREATE TABLE signoff_role_labels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    status_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    color_hex TEXT,
    sort_order INT NOT NULL DEFAULT 0,
    CONSTRAINT signoff_role_labels_uq UNIQUE (tenant_id, status_key)
);
