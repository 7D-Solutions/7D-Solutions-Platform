-- Inventory: Cycle Count Task + Line Schema
--
-- A cycle count task is a workflow wrapper around physical stock counts.
-- Stock changes are NOT applied here — adjustments are created on submit (bd-1q0j).
--
-- Scopes:
--   full    = count all items currently on-hand at the given location
--   partial = count a caller-specified subset of items
--
-- Status machine:
--   open → submitted → approved
--   open | submitted → cancelled
--
-- Depends on: 012 (locations), 001 (items), 005 (item_on_hand)

CREATE TYPE cycle_count_scope AS ENUM ('partial', 'full');
CREATE TYPE cycle_count_status AS ENUM ('open', 'submitted', 'approved', 'cancelled');

CREATE TABLE cycle_count_tasks (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id    TEXT NOT NULL,
    warehouse_id UUID NOT NULL,
    location_id  UUID NOT NULL REFERENCES locations(id),
    scope        cycle_count_scope NOT NULL,
    status       cycle_count_status NOT NULL DEFAULT 'open',
    created_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_cycle_tasks_tenant
    ON cycle_count_tasks(tenant_id, warehouse_id);

CREATE INDEX idx_cycle_tasks_open_location
    ON cycle_count_tasks(location_id)
    WHERE status = 'open';

-- Lines represent one item-per-row within a task.
-- expected_qty is snapshotted from item_on_hand at task creation time.
-- counted_qty is filled in during the submit step (bd-1q0j); NULL = not yet counted.
CREATE TABLE cycle_count_lines (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id      UUID NOT NULL REFERENCES cycle_count_tasks(id),
    tenant_id    TEXT NOT NULL,
    item_id      UUID NOT NULL REFERENCES items(id),
    expected_qty BIGINT NOT NULL DEFAULT 0,
    counted_qty  BIGINT,
    created_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT cycle_count_lines_unique UNIQUE (task_id, item_id)
);

CREATE INDEX idx_cycle_lines_task ON cycle_count_lines(task_id);
