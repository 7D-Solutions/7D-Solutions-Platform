-- Subscription plans
CREATE TABLE subscription_plans (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id VARCHAR(255) NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    schedule VARCHAR(50) NOT NULL CHECK (schedule IN ('weekly', 'monthly', 'custom')),
    price_minor BIGINT NOT NULL,
    currency VARCHAR(3) NOT NULL,
    proration_enabled BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_subscription_plans_tenant ON subscription_plans(tenant_id);

-- Subscriptions
CREATE TABLE subscriptions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id VARCHAR(255) NOT NULL,
    ar_customer_id VARCHAR(255) NOT NULL,
    plan_id UUID NOT NULL REFERENCES subscription_plans(id),
    status VARCHAR(50) NOT NULL CHECK (status IN ('active', 'paused', 'cancelled')),
    schedule VARCHAR(50) NOT NULL CHECK (schedule IN ('weekly', 'monthly', 'custom')),
    price_minor BIGINT NOT NULL,
    currency VARCHAR(3) NOT NULL,
    start_date DATE NOT NULL,
    next_bill_date DATE NOT NULL,
    paused_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_subscriptions_tenant ON subscriptions(tenant_id);
CREATE INDEX idx_subscriptions_ar_customer ON subscriptions(ar_customer_id);
CREATE INDEX idx_subscriptions_status ON subscriptions(status);
CREATE INDEX idx_subscriptions_next_bill_date ON subscriptions(next_bill_date) WHERE status = 'active';

-- Bill runs
CREATE TABLE bill_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bill_run_id VARCHAR(255) NOT NULL UNIQUE,
    execution_date DATE NOT NULL,
    subscriptions_processed INT NOT NULL DEFAULT 0,
    invoices_created INT NOT NULL DEFAULT 0,
    failures INT NOT NULL DEFAULT 0,
    status VARCHAR(50) NOT NULL CHECK (status IN ('running', 'completed', 'failed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_bill_runs_status ON bill_runs(status);
CREATE INDEX idx_bill_runs_execution_date ON bill_runs(execution_date);
