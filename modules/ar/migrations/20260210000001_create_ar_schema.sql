-- AR (Accounts Receivable) Database Schema
-- Full ar_* naming convention (tables, sequences, constraints, indexes)
-- PostgreSQL with SQLx for Rust backend

-- ============================================================
-- ENUMS
-- ============================================================

CREATE TYPE ar_subscriptions_interval AS ENUM (
    'day',
    'week',
    'month',
    'year'
);

CREATE TYPE ar_subscriptions_status AS ENUM (
    'incomplete',
    'incomplete_expired',
    'trialing',
    'active',
    'past_due',
    'canceled',
    'unpaid',
    'paused'
);

CREATE TYPE ar_webhooks_status AS ENUM (
    'received',
    'processing',
    'processed',
    'failed'
);

-- ============================================================
-- CORE TABLES
-- ============================================================

-- Customers
CREATE TABLE ar_customers (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    external_customer_id VARCHAR(255),
    tilled_customer_id VARCHAR(255) UNIQUE,
    status VARCHAR(20) NOT NULL DEFAULT 'active',
    email VARCHAR(255) NOT NULL,
    name VARCHAR(255),
    default_payment_method_id VARCHAR(255),
    payment_method_type VARCHAR(20),
    metadata JSONB,
    update_source VARCHAR(50),
    updated_by VARCHAR(255),
    delinquent_since TIMESTAMP,
    grace_period_end TIMESTAMP,
    next_retry_at TIMESTAMP,
    retry_attempt_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT unique_app_external_customer UNIQUE (app_id, external_customer_id)
);

CREATE INDEX ar_customers_app_id ON ar_customers(app_id);
CREATE INDEX ar_customers_email ON ar_customers(email);
CREATE INDEX ar_customers_delinquent_since ON ar_customers(delinquent_since);
CREATE INDEX ar_customers_next_retry_at ON ar_customers(next_retry_at);

-- Subscriptions
CREATE TABLE ar_subscriptions (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    ar_customer_id INTEGER NOT NULL REFERENCES ar_customers(id) ON DELETE RESTRICT,
    tilled_subscription_id VARCHAR(255) NOT NULL UNIQUE,
    plan_id VARCHAR(100) NOT NULL,
    plan_name VARCHAR(255) NOT NULL,
    price_cents INTEGER NOT NULL,
    status ar_subscriptions_status NOT NULL,
    interval_unit ar_subscriptions_interval NOT NULL,
    interval_count INTEGER NOT NULL DEFAULT 1,
    billing_cycle_anchor TIMESTAMP,
    current_period_start TIMESTAMP NOT NULL,
    current_period_end TIMESTAMP NOT NULL,
    cancel_at_period_end BOOLEAN NOT NULL DEFAULT false,
    cancel_at TIMESTAMP,
    canceled_at TIMESTAMP,
    ended_at TIMESTAMP,
    payment_method_id VARCHAR(255) NOT NULL,
    payment_method_type VARCHAR(20) NOT NULL,
    metadata JSONB,
    update_source VARCHAR(50),
    updated_by VARCHAR(255),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX ar_subscriptions_app_id ON ar_subscriptions(app_id);
CREATE INDEX ar_subscriptions_customer_id ON ar_subscriptions(ar_customer_id);
CREATE INDEX ar_subscriptions_status ON ar_subscriptions(status);
CREATE INDEX ar_subscriptions_plan_id ON ar_subscriptions(plan_id);
CREATE INDEX ar_subscriptions_current_period_end ON ar_subscriptions(current_period_end);
CREATE INDEX ar_subscriptions_app_status_period_end ON ar_subscriptions(app_id, status, current_period_end);

-- Payment Methods
CREATE TABLE ar_payment_methods (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    ar_customer_id INTEGER NOT NULL REFERENCES ar_customers(id) ON DELETE RESTRICT,
    tilled_payment_method_id VARCHAR(255) NOT NULL UNIQUE,
    status VARCHAR(20) NOT NULL DEFAULT 'active',
    type VARCHAR(20) NOT NULL,
    brand VARCHAR(50),
    last4 VARCHAR(4),
    exp_month INTEGER,
    exp_year INTEGER,
    bank_name VARCHAR(255),
    bank_last4 VARCHAR(4),
    is_default BOOLEAN NOT NULL DEFAULT false,
    metadata JSONB,
    deleted_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX ar_payment_methods_app_id ON ar_payment_methods(app_id);
CREATE INDEX ar_payment_methods_customer_id ON ar_payment_methods(ar_customer_id);
CREATE INDEX ar_payment_methods_customer_default ON ar_payment_methods(ar_customer_id, is_default);

-- Invoices
CREATE TABLE ar_invoices (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    tilled_invoice_id VARCHAR(255) NOT NULL UNIQUE,
    ar_customer_id INTEGER NOT NULL REFERENCES ar_customers(id) ON DELETE RESTRICT,
    subscription_id INTEGER REFERENCES ar_subscriptions(id) ON DELETE SET NULL,
    status VARCHAR(20) NOT NULL,
    amount_cents INTEGER NOT NULL,
    currency VARCHAR(3) NOT NULL DEFAULT 'usd',
    due_at TIMESTAMP,
    paid_at TIMESTAMP,
    hosted_url VARCHAR(500),
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL,
    billing_period_start TIMESTAMP,
    billing_period_end TIMESTAMP,
    line_item_details JSONB,
    compliance_codes JSONB
);

CREATE INDEX ar_invoices_app_id ON ar_invoices(app_id);
CREATE INDEX ar_invoices_customer_id ON ar_invoices(ar_customer_id);
CREATE INDEX ar_invoices_subscription_id ON ar_invoices(subscription_id);
CREATE INDEX ar_invoices_status ON ar_invoices(status);
CREATE INDEX ar_invoices_due_at ON ar_invoices(due_at);
CREATE INDEX ar_invoices_app_status_due_at ON ar_invoices(app_id, status, due_at);

-- Charges
CREATE TABLE ar_charges (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    tilled_charge_id VARCHAR(255) UNIQUE,
    invoice_id INTEGER REFERENCES ar_invoices(id) ON DELETE SET NULL,
    ar_customer_id INTEGER NOT NULL REFERENCES ar_customers(id) ON DELETE RESTRICT,
    subscription_id INTEGER REFERENCES ar_subscriptions(id) ON DELETE SET NULL,
    status VARCHAR(20) NOT NULL,
    amount_cents INTEGER NOT NULL,
    currency VARCHAR(3) NOT NULL DEFAULT 'usd',
    charge_type VARCHAR(50) NOT NULL DEFAULT 'one_time',
    reason VARCHAR(100),
    reference_id VARCHAR(255),
    service_date TIMESTAMP,
    note TEXT,
    metadata JSONB,
    failure_code VARCHAR(50),
    failure_message TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL,
    product_type VARCHAR(50),
    quantity INTEGER DEFAULT 1,
    service_frequency VARCHAR(20),
    weight_amount NUMERIC(10,2),
    location_reference VARCHAR(500),
    CONSTRAINT unique_app_reference_id UNIQUE (app_id, reference_id)
);

CREATE INDEX ar_charges_app_id ON ar_charges(app_id);
CREATE INDEX ar_charges_customer_id ON ar_charges(ar_customer_id);
CREATE INDEX ar_charges_subscription_id ON ar_charges(subscription_id);
CREATE INDEX ar_charges_status ON ar_charges(status);
CREATE INDEX ar_charges_charge_type ON ar_charges(charge_type);
CREATE INDEX ar_charges_reason ON ar_charges(reason);
CREATE INDEX ar_charges_service_date ON ar_charges(service_date);
CREATE INDEX ar_charges_app_created_at ON ar_charges(app_id, created_at);

-- Refunds
CREATE TABLE ar_refunds (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    ar_customer_id INTEGER NOT NULL REFERENCES ar_customers(id) ON DELETE RESTRICT,
    charge_id INTEGER NOT NULL REFERENCES ar_charges(id) ON DELETE RESTRICT,
    tilled_refund_id VARCHAR(255),
    tilled_charge_id VARCHAR(255),
    status VARCHAR(20) NOT NULL,
    amount_cents INTEGER NOT NULL,
    currency VARCHAR(3) NOT NULL DEFAULT 'usd',
    reason VARCHAR(100),
    reference_id VARCHAR(255) NOT NULL,
    note TEXT,
    metadata JSONB,
    failure_code VARCHAR(50),
    failure_message TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT unique_refund_app_reference_id UNIQUE (app_id, reference_id),
    CONSTRAINT tilled_refund_id_app_id UNIQUE (tilled_refund_id, app_id)
);

CREATE INDEX ar_refunds_app_id ON ar_refunds(app_id);
CREATE INDEX ar_refunds_customer_id ON ar_refunds(ar_customer_id);
CREATE INDEX ar_refunds_charge_id ON ar_refunds(charge_id);
CREATE INDEX ar_refunds_status ON ar_refunds(status);

-- Disputes
CREATE TABLE ar_disputes (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    tilled_dispute_id VARCHAR(255) NOT NULL,
    tilled_charge_id VARCHAR(255),
    charge_id INTEGER REFERENCES ar_charges(id) ON DELETE SET NULL,
    status VARCHAR(30) NOT NULL,
    amount_cents INTEGER,
    currency VARCHAR(3),
    reason VARCHAR(255),
    reason_code VARCHAR(50),
    evidence_due_by TIMESTAMP,
    opened_at TIMESTAMP,
    closed_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT tilled_dispute_id_app_id UNIQUE (tilled_dispute_id, app_id)
);

CREATE INDEX ar_disputes_app_id ON ar_disputes(app_id);
CREATE INDEX ar_disputes_charge_id ON ar_disputes(charge_id);
CREATE INDEX ar_disputes_status ON ar_disputes(status);
CREATE INDEX ar_disputes_app_status ON ar_disputes(app_id, status);

-- Plans
CREATE TABLE ar_plans (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    plan_id VARCHAR(100) NOT NULL,
    name VARCHAR(255) NOT NULL,
    interval_unit VARCHAR(20) NOT NULL,
    interval_count INTEGER NOT NULL DEFAULT 1,
    price_cents INTEGER NOT NULL,
    currency VARCHAR(3) NOT NULL DEFAULT 'usd',
    features JSONB,
    active BOOLEAN NOT NULL DEFAULT true,
    version_tag VARCHAR(50),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL,
    CONSTRAINT unique_app_plan_id UNIQUE (app_id, plan_id)
);

CREATE INDEX ar_plans_app_id ON ar_plans(app_id);
CREATE INDEX ar_plans_active ON ar_plans(active);

-- Coupons
CREATE TABLE ar_coupons (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    code VARCHAR(100) NOT NULL,
    coupon_type VARCHAR(20) NOT NULL,
    value INTEGER NOT NULL,
    currency VARCHAR(3),
    duration VARCHAR(20) NOT NULL,
    duration_months INTEGER,
    max_redemptions INTEGER,
    redeem_by TIMESTAMP,
    active BOOLEAN NOT NULL DEFAULT true,
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL,
    product_categories JSONB,
    customer_segments JSONB,
    min_quantity INTEGER,
    max_discount_amount_cents INTEGER,
    seasonal_start_date TIMESTAMP,
    seasonal_end_date TIMESTAMP,
    volume_tiers JSONB,
    referral_tier VARCHAR(50),
    contract_term_months INTEGER,
    stackable BOOLEAN DEFAULT false,
    priority INTEGER DEFAULT 0,
    CONSTRAINT unique_app_code UNIQUE (app_id, code)
);

CREATE INDEX ar_coupons_app_id ON ar_coupons(app_id);
CREATE INDEX ar_coupons_active ON ar_coupons(active);
CREATE INDEX ar_coupons_redeem_by ON ar_coupons(redeem_by);

-- Addons
CREATE TABLE ar_addons (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    addon_id VARCHAR(100) NOT NULL,
    name VARCHAR(255) NOT NULL,
    price_cents INTEGER NOT NULL,
    currency VARCHAR(3) NOT NULL DEFAULT 'usd',
    features JSONB,
    active BOOLEAN NOT NULL DEFAULT true,
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL,
    CONSTRAINT unique_app_addon_id UNIQUE (app_id, addon_id)
);

CREATE INDEX ar_addons_app_id ON ar_addons(app_id);
CREATE INDEX ar_addons_active ON ar_addons(active);

-- Subscription Addons (junction table)
CREATE TABLE ar_subscription_addons (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    subscription_id INTEGER NOT NULL REFERENCES ar_subscriptions(id) ON DELETE RESTRICT,
    addon_id INTEGER NOT NULL REFERENCES ar_addons(id) ON DELETE RESTRICT,
    quantity INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL,
    CONSTRAINT unique_subscription_addon UNIQUE (subscription_id, addon_id)
);

CREATE INDEX ar_subscription_addons_app_id ON ar_subscription_addons(app_id);
CREATE INDEX ar_subscription_addons_subscription_id ON ar_subscription_addons(subscription_id);

-- Tax Rates
CREATE TABLE ar_tax_rates (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    jurisdiction_code VARCHAR(20) NOT NULL,
    tax_type VARCHAR(50) NOT NULL,
    rate NUMERIC(5,4) NOT NULL,
    effective_date TIMESTAMP NOT NULL,
    expiration_date TIMESTAMP,
    description VARCHAR(255),
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL,
    CONSTRAINT unique_tax_rate UNIQUE (app_id, jurisdiction_code, tax_type, effective_date)
);

CREATE INDEX ar_tax_rates_app_jurisdiction ON ar_tax_rates(app_id, jurisdiction_code);
CREATE INDEX ar_tax_rates_effective_date ON ar_tax_rates(effective_date);

-- Tax Calculations
CREATE TABLE ar_tax_calculations (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    invoice_id INTEGER REFERENCES ar_invoices(id) ON DELETE RESTRICT,
    charge_id INTEGER REFERENCES ar_charges(id) ON DELETE RESTRICT,
    tax_rate_id INTEGER NOT NULL REFERENCES ar_tax_rates(id) ON DELETE RESTRICT,
    taxable_amount_cents INTEGER NOT NULL,
    tax_amount_cents INTEGER NOT NULL,
    jurisdiction_code VARCHAR(20) NOT NULL,
    tax_type VARCHAR(50) NOT NULL,
    rate_applied NUMERIC(5,4) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX ar_tax_calculations_app_invoice ON ar_tax_calculations(app_id, invoice_id);
CREATE INDEX ar_tax_calculations_app_charge ON ar_tax_calculations(app_id, charge_id);
CREATE INDEX ar_tax_calculations_tax_rate ON ar_tax_calculations(tax_rate_id);

-- Discount Applications
CREATE TABLE ar_discount_applications (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    invoice_id INTEGER REFERENCES ar_invoices(id) ON DELETE RESTRICT,
    charge_id INTEGER REFERENCES ar_charges(id) ON DELETE RESTRICT,
    coupon_id INTEGER REFERENCES ar_coupons(id) ON DELETE RESTRICT,
    customer_id INTEGER REFERENCES ar_customers(id) ON DELETE RESTRICT,
    discount_type VARCHAR(50) NOT NULL,
    discount_amount_cents INTEGER NOT NULL,
    description VARCHAR(255) NOT NULL,
    quantity INTEGER,
    category VARCHAR(50),
    product_types JSONB,
    metadata JSONB,
    applied_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_by VARCHAR(255),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX ar_discount_applications_app_invoice ON ar_discount_applications(app_id, invoice_id);
CREATE INDEX ar_discount_applications_app_charge ON ar_discount_applications(app_id, charge_id);
CREATE INDEX ar_discount_applications_app_coupon ON ar_discount_applications(app_id, coupon_id);
CREATE INDEX ar_discount_applications_app_customer ON ar_discount_applications(app_id, customer_id);
CREATE INDEX ar_discount_applications_applied_at ON ar_discount_applications(applied_at);

-- Metered Usage
CREATE TABLE ar_metered_usage (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    customer_id INTEGER NOT NULL REFERENCES ar_customers(id) ON DELETE RESTRICT,
    subscription_id INTEGER REFERENCES ar_subscriptions(id) ON DELETE RESTRICT,
    metric_name VARCHAR(100) NOT NULL,
    quantity NUMERIC(10,2) NOT NULL,
    unit_price_cents INTEGER NOT NULL,
    period_start TIMESTAMP NOT NULL,
    period_end TIMESTAMP NOT NULL,
    recorded_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    billed_at TIMESTAMP
);

CREATE INDEX ar_metered_usage_app_customer ON ar_metered_usage(app_id, customer_id);
CREATE INDEX ar_metered_usage_app_subscription ON ar_metered_usage(app_id, subscription_id);
CREATE INDEX ar_metered_usage_period ON ar_metered_usage(period_start, period_end);
CREATE INDEX ar_metered_usage_billed_at ON ar_metered_usage(billed_at);

-- Dunning Config
CREATE TABLE ar_dunning_config (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) UNIQUE,
    grace_period_days INTEGER NOT NULL DEFAULT 3,
    retry_schedule_days JSONB NOT NULL,
    max_retry_attempts INTEGER NOT NULL DEFAULT 3,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL
);

CREATE INDEX ar_dunning_config_app_id ON ar_dunning_config(app_id);

-- Invoice Line Items
CREATE TABLE ar_invoice_line_items (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    invoice_id INTEGER NOT NULL REFERENCES ar_invoices(id) ON DELETE RESTRICT,
    line_item_type VARCHAR(50) NOT NULL,
    description VARCHAR(500) NOT NULL,
    quantity NUMERIC(10,2) NOT NULL,
    unit_price_cents INTEGER NOT NULL,
    amount_cents INTEGER NOT NULL,
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX ar_invoice_line_items_app_invoice ON ar_invoice_line_items(app_id, invoice_id);
CREATE INDEX ar_invoice_line_items_line_item_type ON ar_invoice_line_items(line_item_type);

-- ============================================================
-- INFRASTRUCTURE TABLES
-- ============================================================

-- Webhooks
CREATE TABLE ar_webhooks (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    event_id VARCHAR(255) NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    status ar_webhooks_status NOT NULL DEFAULT 'received',
    error TEXT,
    payload JSONB,
    attempt_count INTEGER NOT NULL DEFAULT 1,
    last_attempt_at TIMESTAMP,
    next_attempt_at TIMESTAMP,
    dead_at TIMESTAMP,
    error_code VARCHAR(50),
    received_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    processed_at TIMESTAMP,
    CONSTRAINT event_id_app_id UNIQUE (event_id, app_id)
);

CREATE INDEX ar_webhooks_app_status ON ar_webhooks(app_id, status);
CREATE INDEX ar_webhooks_event_type ON ar_webhooks(event_type);
CREATE INDEX ar_webhooks_next_attempt_at ON ar_webhooks(next_attempt_at);

-- Webhook Attempts
CREATE TABLE ar_webhook_attempts (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    event_id VARCHAR(255) NOT NULL,
    attempt_number INTEGER NOT NULL DEFAULT 1,
    status VARCHAR(20) NOT NULL,
    next_attempt_at TIMESTAMP,
    error_code VARCHAR(50),
    error_message TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL
);

CREATE INDEX ar_webhook_attempts_app_id ON ar_webhook_attempts(app_id);
CREATE INDEX ar_webhook_attempts_event_id ON ar_webhook_attempts(event_id);
CREATE INDEX ar_webhook_attempts_status ON ar_webhook_attempts(status);
CREATE INDEX ar_webhook_attempts_next_attempt_at ON ar_webhook_attempts(next_attempt_at);

-- Idempotency Keys
CREATE TABLE ar_idempotency_keys (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    idempotency_key VARCHAR(255) NOT NULL,
    request_hash VARCHAR(64) NOT NULL,
    response_body JSONB NOT NULL,
    status_code INTEGER NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL,
    CONSTRAINT unique_app_idempotency_key UNIQUE (app_id, idempotency_key)
);

CREATE INDEX ar_idempotency_keys_app_id ON ar_idempotency_keys(app_id);
CREATE INDEX ar_idempotency_keys_expires_at ON ar_idempotency_keys(expires_at);

-- Events (Audit Log)
CREATE TABLE ar_events (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    source VARCHAR(20) NOT NULL,
    entity_type VARCHAR(50),
    entity_id VARCHAR(255),
    payload JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX ar_events_app_id ON ar_events(app_id);
CREATE INDEX ar_events_event_type ON ar_events(event_type);
CREATE INDEX ar_events_source ON ar_events(source);
CREATE INDEX ar_events_created_at ON ar_events(created_at);

-- Reconciliation Runs
CREATE TABLE ar_reconciliation_runs (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    status VARCHAR(20) NOT NULL,
    started_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    finished_at TIMESTAMP,
    stats JSONB,
    error_message TEXT
);

CREATE INDEX ar_reconciliation_runs_app_id ON ar_reconciliation_runs(app_id);
CREATE INDEX ar_reconciliation_runs_status ON ar_reconciliation_runs(status);
CREATE INDEX ar_reconciliation_runs_started_at ON ar_reconciliation_runs(started_at);

-- Divergences
CREATE TABLE ar_divergences (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    run_id INTEGER NOT NULL REFERENCES ar_reconciliation_runs(id) ON DELETE CASCADE,
    entity_type VARCHAR(50) NOT NULL,
    entity_key VARCHAR(255) NOT NULL,
    divergence_type VARCHAR(50) NOT NULL,
    local_snapshot JSONB,
    remote_snapshot JSONB,
    status VARCHAR(20) NOT NULL DEFAULT 'open',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    resolved_at TIMESTAMP
);

CREATE INDEX ar_divergences_app_id ON ar_divergences(app_id);
CREATE INDEX ar_divergences_run_id ON ar_divergences(run_id);
CREATE INDEX ar_divergences_entity_type ON ar_divergences(entity_type);
CREATE INDEX ar_divergences_status ON ar_divergences(status);
