-- Traveler Holds
-- hold_type: quality/engineering/material/customer/other
-- scope: work_order/operation
-- status: active/released/cancelled
-- release_authority: quality/engineering/planner/supervisor/owner_only/any_with_role

CREATE TABLE traveler_holds (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    hold_number TEXT NOT NULL,
    hold_type TEXT NOT NULL,
    scope TEXT NOT NULL,
    work_order_id UUID NOT NULL,
    operation_id UUID,
    reason TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    release_authority TEXT NOT NULL DEFAULT 'any_with_role',
    placed_by UUID NOT NULL,
    placed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    released_by UUID,
    released_at TIMESTAMPTZ,
    release_notes TEXT,
    cancelled_by UUID,
    cancelled_at TIMESTAMPTZ,
    cancel_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT traveler_holds_status_check CHECK (status IN ('active', 'released', 'cancelled')),
    CONSTRAINT traveler_holds_scope_check CHECK (scope IN ('work_order', 'operation')),
    CONSTRAINT traveler_holds_op_scope CHECK (scope != 'operation' OR operation_id IS NOT NULL)
);

CREATE UNIQUE INDEX idx_traveler_holds_number_tenant ON traveler_holds (tenant_id, hold_number);
CREATE INDEX idx_traveler_holds_tenant ON traveler_holds (tenant_id);
CREATE INDEX idx_traveler_holds_wo ON traveler_holds (tenant_id, work_order_id);
CREATE INDEX idx_traveler_holds_op ON traveler_holds (tenant_id, operation_id) WHERE operation_id IS NOT NULL;
CREATE INDEX idx_traveler_holds_status ON traveler_holds (tenant_id, status);

-- Hold label tables
CREATE TABLE hold_type_labels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    status_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    color_hex TEXT,
    sort_order INT NOT NULL DEFAULT 0,
    CONSTRAINT hold_type_labels_uq UNIQUE (tenant_id, status_key)
);

CREATE TABLE hold_scope_labels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    status_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    color_hex TEXT,
    sort_order INT NOT NULL DEFAULT 0,
    CONSTRAINT hold_scope_labels_uq UNIQUE (tenant_id, status_key)
);

CREATE TABLE hold_release_authority_labels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    status_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    color_hex TEXT,
    sort_order INT NOT NULL DEFAULT 0,
    CONSTRAINT hold_release_authority_labels_uq UNIQUE (tenant_id, status_key)
);

CREATE TABLE hold_status_labels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    status_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    color_hex TEXT,
    sort_order INT NOT NULL DEFAULT 0,
    CONSTRAINT hold_status_labels_uq UNIQUE (tenant_id, status_key)
);
