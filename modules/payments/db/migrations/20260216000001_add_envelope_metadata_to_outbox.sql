-- Add missing envelope metadata columns to payments_events_outbox for Phase 16
-- Extends existing envelope fields with full constitutional metadata

-- Add missing required envelope fields
ALTER TABLE payments_events_outbox
  ADD COLUMN source_module VARCHAR(100),
  ADD COLUMN source_version VARCHAR(50),
  ADD COLUMN schema_version VARCHAR(50),
  ADD COLUMN replay_safe BOOLEAN DEFAULT true;

-- Add missing optional envelope fields
ALTER TABLE payments_events_outbox
  ADD COLUMN trace_id VARCHAR(255),
  ADD COLUMN reverses_event_id UUID,
  ADD COLUMN supersedes_event_id UUID,
  ADD COLUMN side_effect_id VARCHAR(255),
  ADD COLUMN mutation_class VARCHAR(100);

-- Create index on trace_id for distributed tracing
CREATE INDEX payments_events_outbox_trace_id ON payments_events_outbox(trace_id) WHERE trace_id IS NOT NULL;

-- Create index on mutation_class for operational queries
CREATE INDEX payments_events_outbox_mutation_class ON payments_events_outbox(mutation_class) WHERE mutation_class IS NOT NULL;

-- Update table comment
COMMENT ON TABLE payments_events_outbox IS 'Transactional outbox for reliable event publishing with full envelope metadata (Phase 16)';
