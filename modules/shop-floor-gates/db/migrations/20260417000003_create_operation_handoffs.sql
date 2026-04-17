-- Operation Handoffs
-- initiation_type: push/pull
-- status: initiated/accepted/rejected/cancelled

CREATE TABLE operation_handoffs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    handoff_number TEXT NOT NULL,
    work_order_id UUID NOT NULL,
    source_operation_id UUID NOT NULL,
    dest_operation_id UUID NOT NULL,
    initiation_type TEXT NOT NULL DEFAULT 'push',
    status TEXT NOT NULL DEFAULT 'initiated',
    quantity NUMERIC(18,6) NOT NULL,
    unit_of_measure TEXT NOT NULL,
    lot_number TEXT,
    serial_numbers TEXT[],
    notes TEXT,
    initiated_by UUID NOT NULL,
    initiated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    accepted_by UUID,
    accepted_at TIMESTAMPTZ,
    rejected_by UUID,
    rejected_at TIMESTAMPTZ,
    rejection_reason TEXT,
    cancelled_by UUID,
    cancelled_at TIMESTAMPTZ,
    cancel_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT operation_handoffs_status_check CHECK (status IN ('initiated', 'accepted', 'rejected', 'cancelled')),
    CONSTRAINT operation_handoffs_init_type_check CHECK (initiation_type IN ('push', 'pull'))
);

CREATE UNIQUE INDEX idx_operation_handoffs_number_tenant ON operation_handoffs (tenant_id, handoff_number);
CREATE INDEX idx_operation_handoffs_tenant ON operation_handoffs (tenant_id);
CREATE INDEX idx_operation_handoffs_wo ON operation_handoffs (tenant_id, work_order_id);
CREATE INDEX idx_operation_handoffs_src_op ON operation_handoffs (tenant_id, source_operation_id);
CREATE INDEX idx_operation_handoffs_dest_op ON operation_handoffs (tenant_id, dest_operation_id);
CREATE INDEX idx_operation_handoffs_status ON operation_handoffs (tenant_id, status);

-- Label tables
CREATE TABLE handoff_initiation_labels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    status_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    color_hex TEXT,
    sort_order INT NOT NULL DEFAULT 0,
    CONSTRAINT handoff_initiation_labels_uq UNIQUE (tenant_id, status_key)
);

CREATE TABLE handoff_status_labels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    status_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    color_hex TEXT,
    sort_order INT NOT NULL DEFAULT 0,
    CONSTRAINT handoff_status_labels_uq UNIQUE (tenant_id, status_key)
);
