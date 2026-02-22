-- Checkout sessions table for customer-facing payment flows
-- Tracks PaymentIntents created in requires_payment_method state (Tilled.js flow)

CREATE TABLE checkout_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    invoice_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    amount_minor INTEGER NOT NULL,
    currency TEXT NOT NULL,
    -- Tilled PaymentIntent ID
    processor_payment_id TEXT NOT NULL,
    -- client_secret returned to Tilled.js for browser-side payment completion
    client_secret TEXT NOT NULL,
    return_url TEXT,
    cancel_url TEXT,
    -- pending | succeeded | failed | cancelled
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_checkout_sessions_processor_id ON checkout_sessions(processor_payment_id);
CREATE INDEX idx_checkout_sessions_invoice_id ON checkout_sessions(invoice_id);
CREATE INDEX idx_checkout_sessions_tenant_id ON checkout_sessions(tenant_id);
