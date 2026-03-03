CREATE TABLE IF NOT EXISTS portal_document_links (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    party_id UUID NOT NULL,
    document_id UUID NOT NULL,
    display_title TEXT,
    created_by UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, party_id, document_id)
);

CREATE TABLE IF NOT EXISTS portal_status_feed (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    party_id UUID NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id UUID,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    source TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_portal_status_feed_party_time
    ON portal_status_feed (tenant_id, party_id, occurred_at DESC);

CREATE TABLE IF NOT EXISTS portal_acknowledgments (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    party_id UUID NOT NULL,
    portal_user_id UUID NOT NULL REFERENCES portal_users(id) ON DELETE RESTRICT,
    document_id UUID,
    status_card_id UUID,
    ack_type TEXT NOT NULL,
    notes TEXT,
    idempotency_key TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, party_id, idempotency_key)
);
