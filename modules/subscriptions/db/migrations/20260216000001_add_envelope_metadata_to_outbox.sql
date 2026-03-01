-- Add envelope metadata columns to events_outbox for Phase 16
-- These columns make envelope metadata queryable and support deterministic replay analysis

-- Required envelope fields
ALTER TABLE events_outbox
  ADD COLUMN event_id UUID,
  ADD COLUMN event_type VARCHAR(255),
  ADD COLUMN tenant_id VARCHAR(255),
  ADD COLUMN source_module VARCHAR(100),
  ADD COLUMN source_version VARCHAR(50),
  ADD COLUMN schema_version VARCHAR(50),
  ADD COLUMN replay_safe BOOLEAN DEFAULT true;

-- Optional envelope fields
ALTER TABLE events_outbox
  ADD COLUMN trace_id VARCHAR(255),
  ADD COLUMN correlation_id VARCHAR(255),
  ADD COLUMN causation_id VARCHAR(255),
  ADD COLUMN reverses_event_id UUID,
  ADD COLUMN supersedes_event_id UUID,
  ADD COLUMN side_effect_id VARCHAR(255),
  ADD COLUMN mutation_class VARCHAR(100);

-- Add occurred_at from envelope (different from created_at which is when row was inserted)
ALTER TABLE events_outbox
  ADD COLUMN occurred_at TIMESTAMPTZ;

-- Create index on event_id for deduplication and lookups
CREATE INDEX idx_events_outbox_event_id ON events_outbox(event_id);

-- Create index on tenant_id for tenant-scoped queries
CREATE INDEX idx_events_outbox_tenant_id ON events_outbox(tenant_id);

-- Create index on mutation_class for operational queries
CREATE INDEX idx_events_outbox_mutation_class ON events_outbox(mutation_class) WHERE mutation_class IS NOT NULL;

-- Create index on trace_id for distributed tracing
CREATE INDEX idx_events_outbox_trace_id ON events_outbox(trace_id) WHERE trace_id IS NOT NULL;

-- Comment on table
COMMENT ON TABLE events_outbox IS 'Transactional outbox for reliable event publishing with full envelope metadata';
