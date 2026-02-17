-- Platform-level Audit Log Schema
-- Append-only audit log for tracking all mutations across the platform
-- PostgreSQL with SQLx for Rust backend

-- ============================================================
-- ENUMS
-- ============================================================

DO $$
BEGIN
    CREATE TYPE mutation_class AS ENUM (
        'CREATE',
        'UPDATE',
        'DELETE',
        'STATE_TRANSITION',
        'REVERSAL'
    );
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

-- ============================================================
-- AUDIT LOG TABLE (APPEND-ONLY)
-- ============================================================

CREATE TABLE IF NOT EXISTS audit_events (
    audit_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Actor context
    actor_id UUID NOT NULL,
    actor_type VARCHAR(50) NOT NULL,

    -- Action details
    action VARCHAR(100) NOT NULL,
    mutation_class mutation_class NOT NULL,

    -- Entity identification
    entity_type VARCHAR(100) NOT NULL,
    entity_id VARCHAR(255) NOT NULL,

    -- State snapshots
    before_snapshot JSONB,
    after_snapshot JSONB,
    before_hash VARCHAR(64),
    after_hash VARCHAR(64),

    -- Event correlation
    causation_id UUID,
    correlation_id UUID,
    trace_id VARCHAR(64),

    -- Metadata
    metadata JSONB,

    -- Constraints
    CONSTRAINT audit_id_immutable CHECK (audit_id IS NOT NULL)
);

-- ============================================================
-- INDEXES FOR QUERY PERFORMANCE
-- ============================================================

CREATE INDEX IF NOT EXISTS audit_events_occurred_at ON audit_events(occurred_at DESC);
CREATE INDEX IF NOT EXISTS audit_events_actor_id ON audit_events(actor_id);
CREATE INDEX IF NOT EXISTS audit_events_entity ON audit_events(entity_type, entity_id);
CREATE INDEX IF NOT EXISTS audit_events_action ON audit_events(action);
CREATE INDEX IF NOT EXISTS audit_events_mutation_class ON audit_events(mutation_class);
CREATE INDEX IF NOT EXISTS audit_events_correlation_id ON audit_events(correlation_id);
CREATE INDEX IF NOT EXISTS audit_events_trace_id ON audit_events(trace_id);

-- ============================================================
-- APPEND-ONLY ENFORCEMENT
-- ============================================================

-- Disable UPDATE and DELETE operations on audit_events table
CREATE OR REPLACE FUNCTION prevent_audit_modification()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'Audit log is append-only: UPDATE and DELETE operations are forbidden';
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE TRIGGER enforce_append_only_update
    BEFORE UPDATE ON audit_events
    FOR EACH ROW
    EXECUTE FUNCTION prevent_audit_modification();

CREATE OR REPLACE TRIGGER enforce_append_only_delete
    BEFORE DELETE ON audit_events
    FOR EACH ROW
    EXECUTE FUNCTION prevent_audit_modification();

-- ============================================================
-- COMMENTS
-- ============================================================

COMMENT ON TABLE audit_events IS 'Platform-wide append-only audit log for all mutations';
COMMENT ON COLUMN audit_events.audit_id IS 'Unique identifier for this audit event';
COMMENT ON COLUMN audit_events.occurred_at IS 'Timestamp when the mutation occurred';
COMMENT ON COLUMN audit_events.actor_id IS 'ID of the user/service/system that performed the action';
COMMENT ON COLUMN audit_events.actor_type IS 'Type of actor (User, Service, System)';
COMMENT ON COLUMN audit_events.action IS 'Human-readable description of the action';
COMMENT ON COLUMN audit_events.mutation_class IS 'Classification of the mutation';
COMMENT ON COLUMN audit_events.entity_type IS 'Type of entity being mutated';
COMMENT ON COLUMN audit_events.entity_id IS 'ID of the entity being mutated';
COMMENT ON COLUMN audit_events.before_snapshot IS 'JSON snapshot of entity state before mutation';
COMMENT ON COLUMN audit_events.after_snapshot IS 'JSON snapshot of entity state after mutation';
COMMENT ON COLUMN audit_events.before_hash IS 'Hash of before state for integrity verification';
COMMENT ON COLUMN audit_events.after_hash IS 'Hash of after state for integrity verification';
COMMENT ON COLUMN audit_events.causation_id IS 'ID of the event that caused this mutation';
COMMENT ON COLUMN audit_events.correlation_id IS 'ID linking related events across modules';
COMMENT ON COLUMN audit_events.trace_id IS 'Distributed tracing ID';
COMMENT ON COLUMN audit_events.metadata IS 'Additional context metadata';
