-- Add envelope metadata columns to events_outbox (bd-24o3)
-- Phase 16: Envelope Metadata Queryability

-- Add required envelope metadata columns (nullable for backward compatibility)
ALTER TABLE events_outbox
    ADD COLUMN IF NOT EXISTS event_id UUID,
    ADD COLUMN IF NOT EXISTS event_type TEXT,
    ADD COLUMN IF NOT EXISTS tenant_id TEXT,
    ADD COLUMN IF NOT EXISTS source_module TEXT,
    ADD COLUMN IF NOT EXISTS source_version TEXT,
    ADD COLUMN IF NOT EXISTS schema_version TEXT,
    ADD COLUMN IF NOT EXISTS replay_safe BOOLEAN,
    ADD COLUMN IF NOT EXISTS occurred_at TIMESTAMPTZ;

-- Add optional envelope metadata columns (for tracing, linkage, classification)
ALTER TABLE events_outbox
    ADD COLUMN IF NOT EXISTS trace_id TEXT,
    ADD COLUMN IF NOT EXISTS correlation_id TEXT,
    ADD COLUMN IF NOT EXISTS causation_id TEXT,
    ADD COLUMN IF NOT EXISTS reverses_event_id UUID,
    ADD COLUMN IF NOT EXISTS supersedes_event_id UUID,
    ADD COLUMN IF NOT EXISTS side_effect_id TEXT,
    ADD COLUMN IF NOT EXISTS mutation_class TEXT;

-- Create indexes for operational queries
CREATE INDEX IF NOT EXISTS idx_events_outbox_event_id ON events_outbox(event_id);
CREATE INDEX IF NOT EXISTS idx_events_outbox_tenant_id ON events_outbox(tenant_id);
CREATE INDEX IF NOT EXISTS idx_events_outbox_trace_id ON events_outbox(trace_id) WHERE trace_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_events_outbox_mutation_class ON events_outbox(mutation_class) WHERE mutation_class IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_events_outbox_occurred_at ON events_outbox(occurred_at);

-- Add comment for documentation
COMMENT ON TABLE events_outbox IS 'Transactional outbox for reliable event publishing. Contains EventEnvelope metadata for deterministic replay.';
