-- Inventory: Barcode Format Rules
--
-- barcode_format_rules:
--   Tenant-defined regex patterns for resolving raw barcode strings to
--   typed entity references. Rules are evaluated in ascending priority order
--   (lowest number = highest precedence); ties broken by rule id (stable).
--   Deactivated rules are never evaluated.
--
-- entity_type_when_matched:
--   Canonical values: work_order, operation, item, lot, serial, badge, other
--   For Inventory-native types (item/lot/serial), the resolver looks up the
--   decoded reference in Inventory tables.
--   For cross-module types (work_order/operation/badge/other), the resolver
--   returns the captured reference string for the caller to validate.
--
-- capture_group_index:
--   Which capture group in the regex contains the entity key (0 = full match).

CREATE TABLE barcode_format_rules (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id               TEXT NOT NULL,
    rule_name               TEXT NOT NULL,
    -- Tenant-supplied regex (validated to compile at insert time)
    pattern_regex           TEXT NOT NULL,
    entity_type_when_matched TEXT NOT NULL,
    -- 0 = full match, 1 = first capture group, etc.
    capture_group_index     INT NOT NULL DEFAULT 0,
    -- Lower number = evaluated first. Ties broken by id.
    priority                INT NOT NULL DEFAULT 100,
    active                  BOOLEAN NOT NULL DEFAULT TRUE,
    created_at              TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    -- User or system principal that last modified this rule
    updated_by              TEXT,

    CONSTRAINT barcode_rules_entity_type_check CHECK (
        entity_type_when_matched IN (
            'work_order', 'operation', 'item', 'lot', 'serial', 'badge', 'other'
        )
    ),
    CONSTRAINT barcode_rules_capture_group_nn CHECK (capture_group_index >= 0)
);

CREATE INDEX idx_barcode_rules_tenant_active_priority
    ON barcode_format_rules(tenant_id, active, priority, id)
    WHERE active = TRUE;

CREATE INDEX idx_barcode_rules_tenant
    ON barcode_format_rules(tenant_id);
