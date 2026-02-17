-- Add envelope metadata columns to events_outbox for Phase 16
-- Extends existing fields with full constitutional envelope metadata
-- NOTE: Uses IF NOT EXISTS / ADD COLUMN IF NOT EXISTS for idempotency

-- Add required envelope fields
ALTER TABLE events_outbox
  ADD COLUMN IF NOT EXISTS tenant_id VARCHAR(255),
  ADD COLUMN IF NOT EXISTS source_module VARCHAR(100),
  ADD COLUMN IF NOT EXISTS source_version VARCHAR(50),
  ADD COLUMN IF NOT EXISTS schema_version VARCHAR(50),
  ADD COLUMN IF NOT EXISTS occurred_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS replay_safe BOOLEAN DEFAULT true;

-- Add optional envelope fields
ALTER TABLE events_outbox
  ADD COLUMN IF NOT EXISTS trace_id VARCHAR(255),
  ADD COLUMN IF NOT EXISTS correlation_id VARCHAR(255),
  ADD COLUMN IF NOT EXISTS causation_id VARCHAR(255),
  ADD COLUMN IF NOT EXISTS reverses_event_id UUID,
  ADD COLUMN IF NOT EXISTS supersedes_event_id UUID,
  ADD COLUMN IF NOT EXISTS side_effect_id VARCHAR(255),
  ADD COLUMN IF NOT EXISTS mutation_class VARCHAR(100);

-- Create index on tenant_id for tenant-scoped queries
CREATE INDEX IF NOT EXISTS idx_events_outbox_tenant_id ON events_outbox(tenant_id) WHERE tenant_id IS NOT NULL;

-- Create index on trace_id for distributed tracing
CREATE INDEX IF NOT EXISTS idx_events_outbox_trace_id ON events_outbox(trace_id) WHERE trace_id IS NOT NULL;

-- Create index on mutation_class for operational queries (critical for GL postings/reversals)
CREATE INDEX IF NOT EXISTS idx_events_outbox_mutation_class ON events_outbox(mutation_class) WHERE mutation_class IS NOT NULL;

-- Create index on occurred_at for event ordering
CREATE INDEX IF NOT EXISTS idx_events_outbox_occurred_at ON events_outbox(occurred_at) WHERE occurred_at IS NOT NULL;

-- Create index on reverses_event_id for reversal tracking (GL-specific usecase)
CREATE INDEX IF NOT EXISTS idx_events_outbox_reverses_event_id ON events_outbox(reverses_event_id) WHERE reverses_event_id IS NOT NULL;

-- Update table comment
COMMENT ON TABLE events_outbox IS 'Transactional outbox for reliable event publishing with full envelope metadata (Phase 16) - supports GL posting/reversal tracing';
