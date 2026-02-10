-- AR (Accounts Receivable) Database Schema
-- Generated from Prisma schema for PostgreSQL
-- SQLx migration for Rust backend

-- ============================================================
-- ENUMS
-- ============================================================

CREATE TYPE billing_subscriptions_status AS ENUM (
    'incomplete',
    'incomplete_expired',
    'trialing',
    'active',
    'past_due',
    'canceled',
    'unpaid',
    'paused'
);

CREATE TYPE billing_subscriptions_interval AS ENUM (
    'day',
    'week',
    'month',
    'year'
);

CREATE TYPE billing_webhooks_status AS ENUM (
    'received',
    'processing',
    'processed',
    'failed'
);

-- ============================================================
-- PHASE 1: CORE BILLING TABLES
-- ============================================================

CREATE TABLE IF NOT EXISTS billing_customers (
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

CREATE TABLE IF NOT EXISTS billing_subscriptions (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    billing_customer_id INTEGER NOT NULL,
    tilled_subscription_id VARCHAR(255) NOT NULL UNIQUE,
    plan_id VARCHAR(100) NOT NULL,
    plan_name VARCHAR(255) NOT NULL,
    price_cents INTEGER NOT NULL,
    status billing_subscriptions_status NOT NULL,
    interval_unit billing_subscriptions_interval NOT NULL,
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
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (billing_customer_id) REFERENCES billing_customers(id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS billing_payment_methods (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    billing_customer_id INTEGER NOT NULL,
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
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (billing_customer_id) REFERENCES billing_customers(id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS billing_webhooks (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    event_id VARCHAR(255) NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    status billing_webhooks_status NOT NULL DEFAULT 'received',
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

-- ============================================================
-- PHASE 2: RELIABILITY & SAFETY TABLES
-- ============================================================

CREATE TABLE IF NOT EXISTS billing_idempotency_keys (
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

CREATE TABLE IF NOT EXISTS billing_events (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    source VARCHAR(20) NOT NULL,
    entity_type VARCHAR(50),
    entity_id VARCHAR(255),
    payload JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS billing_webhook_attempts (
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

CREATE TABLE IF NOT EXISTS billing_reconciliation_runs (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    status VARCHAR(20) NOT NULL,
    started_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    finished_at TIMESTAMP,
    stats JSONB,
    error_message TEXT
);

CREATE TABLE IF NOT EXISTS billing_divergences (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    run_id INTEGER NOT NULL,
    entity_type VARCHAR(50) NOT NULL,
    entity_key VARCHAR(255) NOT NULL,
    divergence_type VARCHAR(50) NOT NULL,
    local_snapshot JSONB,
    remote_snapshot JSONB,
    status VARCHAR(20) NOT NULL DEFAULT 'open',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    resolved_at TIMESTAMP,

    FOREIGN KEY (run_id) REFERENCES billing_reconciliation_runs(id) ON DELETE CASCADE
);

-- ============================================================
-- PHASE 3: PRICING AGILITY TABLES
-- ============================================================

CREATE TABLE IF NOT EXISTS billing_plans (
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

CREATE TABLE IF NOT EXISTS billing_coupons (
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
    -- Generic discount fields
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

CREATE TABLE IF NOT EXISTS billing_addons (
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

CREATE TABLE IF NOT EXISTS billing_subscription_addons (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    subscription_id INTEGER NOT NULL,
    addon_id INTEGER NOT NULL,
    quantity INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL,

    CONSTRAINT unique_subscription_addon UNIQUE (subscription_id, addon_id),
    FOREIGN KEY (subscription_id) REFERENCES billing_subscriptions(id) ON DELETE RESTRICT,
    FOREIGN KEY (addon_id) REFERENCES billing_addons(id) ON DELETE RESTRICT
);

-- ============================================================
-- PHASE 4: MONEY RECORDS TABLES
-- ============================================================

CREATE TABLE IF NOT EXISTS billing_invoices (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    tilled_invoice_id VARCHAR(255) NOT NULL UNIQUE,
    billing_customer_id INTEGER NOT NULL,
    subscription_id INTEGER,
    status VARCHAR(20) NOT NULL,
    amount_cents INTEGER NOT NULL,
    currency VARCHAR(3) NOT NULL DEFAULT 'usd',
    due_at TIMESTAMP,
    paid_at TIMESTAMP,
    hosted_url VARCHAR(500),
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL,
    -- Generic invoice fields
    billing_period_start TIMESTAMP,
    billing_period_end TIMESTAMP,
    line_item_details JSONB,
    compliance_codes JSONB,

    FOREIGN KEY (billing_customer_id) REFERENCES billing_customers(id) ON DELETE RESTRICT,
    FOREIGN KEY (subscription_id) REFERENCES billing_subscriptions(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS billing_charges (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    tilled_charge_id VARCHAR(255) UNIQUE,
    invoice_id INTEGER,
    billing_customer_id INTEGER NOT NULL,
    subscription_id INTEGER,
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
    -- Generic charge fields
    product_type VARCHAR(50),
    quantity INTEGER DEFAULT 1,
    service_frequency VARCHAR(20),
    weight_amount DECIMAL(10,2),
    location_reference VARCHAR(500),

    CONSTRAINT unique_app_reference_id UNIQUE (app_id, reference_id),
    FOREIGN KEY (invoice_id) REFERENCES billing_invoices(id) ON DELETE SET NULL,
    FOREIGN KEY (billing_customer_id) REFERENCES billing_customers(id) ON DELETE RESTRICT,
    FOREIGN KEY (subscription_id) REFERENCES billing_subscriptions(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS billing_refunds (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    billing_customer_id INTEGER NOT NULL,
    charge_id INTEGER NOT NULL,
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
    CONSTRAINT tilled_refund_id_app_id UNIQUE (tilled_refund_id, app_id),
    FOREIGN KEY (charge_id) REFERENCES billing_charges(id) ON DELETE RESTRICT,
    FOREIGN KEY (billing_customer_id) REFERENCES billing_customers(id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS billing_disputes (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    tilled_dispute_id VARCHAR(255) NOT NULL,
    tilled_charge_id VARCHAR(255),
    charge_id INTEGER,
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

    CONSTRAINT tilled_dispute_id_app_id UNIQUE (tilled_dispute_id, app_id),
    FOREIGN KEY (charge_id) REFERENCES billing_charges(id) ON DELETE SET NULL
);

-- ============================================================
-- TAX ENGINE TABLES
-- ============================================================

CREATE TABLE IF NOT EXISTS billing_tax_rates (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    jurisdiction_code VARCHAR(20) NOT NULL,
    tax_type VARCHAR(50) NOT NULL,
    rate DECIMAL(5,4) NOT NULL,
    effective_date TIMESTAMP NOT NULL,
    expiration_date TIMESTAMP,
    description VARCHAR(255),
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL,

    CONSTRAINT unique_tax_rate UNIQUE (app_id, jurisdiction_code, tax_type, effective_date)
);

CREATE TABLE IF NOT EXISTS billing_tax_calculations (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    invoice_id INTEGER,
    charge_id INTEGER,
    tax_rate_id INTEGER NOT NULL,
    taxable_amount_cents INTEGER NOT NULL,
    tax_amount_cents INTEGER NOT NULL,
    jurisdiction_code VARCHAR(20) NOT NULL,
    tax_type VARCHAR(50) NOT NULL,
    rate_applied DECIMAL(5,4) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (invoice_id) REFERENCES billing_invoices(id) ON DELETE RESTRICT,
    FOREIGN KEY (charge_id) REFERENCES billing_charges(id) ON DELETE RESTRICT,
    FOREIGN KEY (tax_rate_id) REFERENCES billing_tax_rates(id) ON DELETE RESTRICT
);

-- ============================================================
-- DISCOUNT APPLICATIONS
-- ============================================================

CREATE TABLE IF NOT EXISTS billing_discount_applications (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    invoice_id INTEGER,
    charge_id INTEGER,
    coupon_id INTEGER,
    customer_id INTEGER,
    discount_type VARCHAR(50) NOT NULL,
    discount_amount_cents INTEGER NOT NULL,
    description VARCHAR(255) NOT NULL,
    quantity INTEGER,
    category VARCHAR(50),
    product_types JSONB,
    metadata JSONB,
    applied_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_by VARCHAR(255),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (invoice_id) REFERENCES billing_invoices(id) ON DELETE RESTRICT,
    FOREIGN KEY (charge_id) REFERENCES billing_charges(id) ON DELETE RESTRICT,
    FOREIGN KEY (coupon_id) REFERENCES billing_coupons(id) ON DELETE RESTRICT,
    FOREIGN KEY (customer_id) REFERENCES billing_customers(id) ON DELETE RESTRICT
);

-- ============================================================
-- METERED USAGE
-- ============================================================

CREATE TABLE IF NOT EXISTS billing_metered_usage (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    customer_id INTEGER NOT NULL,
    subscription_id INTEGER,
    metric_name VARCHAR(100) NOT NULL,
    quantity DECIMAL(10,2) NOT NULL,
    unit_price_cents INTEGER NOT NULL,
    period_start TIMESTAMP NOT NULL,
    period_end TIMESTAMP NOT NULL,
    recorded_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    billed_at TIMESTAMP,

    FOREIGN KEY (customer_id) REFERENCES billing_customers(id) ON DELETE RESTRICT,
    FOREIGN KEY (subscription_id) REFERENCES billing_subscriptions(id) ON DELETE RESTRICT
);

-- ============================================================
-- DUNNING CONFIGURATION
-- ============================================================

CREATE TABLE IF NOT EXISTS billing_dunning_config (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50),
    grace_period_days INTEGER NOT NULL DEFAULT 3,
    retry_schedule_days JSONB NOT NULL,
    max_retry_attempts INTEGER NOT NULL DEFAULT 3,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL,

    CONSTRAINT unique_app_dunning_config UNIQUE (app_id)
);

-- ============================================================
-- INVOICE LINE ITEMS
-- ============================================================

CREATE TABLE IF NOT EXISTS billing_invoice_line_items (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    invoice_id INTEGER NOT NULL,
    line_item_type VARCHAR(50) NOT NULL,
    description VARCHAR(500) NOT NULL,
    quantity DECIMAL(10,2) NOT NULL,
    unit_price_cents INTEGER NOT NULL,
    amount_cents INTEGER NOT NULL,
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (invoice_id) REFERENCES billing_invoices(id) ON DELETE RESTRICT
);

-- ============================================================
-- INDEXES
-- ============================================================

-- Customers
CREATE INDEX IF NOT EXISTS ar_customers_app_id ON billing_customers(app_id);
CREATE INDEX IF NOT EXISTS ar_customers_email ON billing_customers(email);
CREATE INDEX IF NOT EXISTS ar_customers_delinquent_since ON billing_customers(delinquent_since);
CREATE INDEX IF NOT EXISTS ar_customers_next_retry_at ON billing_customers(next_retry_at);

-- Subscriptions
CREATE INDEX IF NOT EXISTS ar_subscriptions_app_id ON billing_subscriptions(app_id);
CREATE INDEX IF NOT EXISTS ar_subscriptions_customer_id ON billing_subscriptions(billing_customer_id);
CREATE INDEX IF NOT EXISTS ar_subscriptions_status ON billing_subscriptions(status);
CREATE INDEX IF NOT EXISTS ar_subscriptions_plan_id ON billing_subscriptions(plan_id);
CREATE INDEX IF NOT EXISTS ar_subscriptions_current_period_end ON billing_subscriptions(current_period_end);
CREATE INDEX IF NOT EXISTS ar_subscriptions_app_status_period_end ON billing_subscriptions(app_id, status, current_period_end);

-- Payment Methods
CREATE INDEX IF NOT EXISTS ar_payment_methods_app_id ON billing_payment_methods(app_id);
CREATE INDEX IF NOT EXISTS ar_payment_methods_customer_id ON billing_payment_methods(billing_customer_id);
CREATE INDEX IF NOT EXISTS ar_payment_methods_customer_default ON billing_payment_methods(billing_customer_id, is_default);

-- Webhooks
CREATE INDEX IF NOT EXISTS ar_webhooks_app_status ON billing_webhooks(app_id, status);
CREATE INDEX IF NOT EXISTS ar_webhooks_event_type ON billing_webhooks(event_type);
CREATE INDEX IF NOT EXISTS ar_webhooks_next_attempt_at ON billing_webhooks(next_attempt_at);

-- Idempotency Keys
CREATE INDEX IF NOT EXISTS ar_idempotency_keys_app_id ON billing_idempotency_keys(app_id);
CREATE INDEX IF NOT EXISTS ar_idempotency_keys_expires_at ON billing_idempotency_keys(expires_at);

-- Events
CREATE INDEX IF NOT EXISTS ar_events_app_id ON billing_events(app_id);
CREATE INDEX IF NOT EXISTS ar_events_event_type ON billing_events(event_type);
CREATE INDEX IF NOT EXISTS ar_events_source ON billing_events(source);
CREATE INDEX IF NOT EXISTS ar_events_created_at ON billing_events(created_at);

-- Webhook Attempts
CREATE INDEX IF NOT EXISTS ar_webhook_attempts_app_id ON billing_webhook_attempts(app_id);
CREATE INDEX IF NOT EXISTS ar_webhook_attempts_event_id ON billing_webhook_attempts(event_id);
CREATE INDEX IF NOT EXISTS ar_webhook_attempts_status ON billing_webhook_attempts(status);
CREATE INDEX IF NOT EXISTS ar_webhook_attempts_next_attempt_at ON billing_webhook_attempts(next_attempt_at);

-- Reconciliation
CREATE INDEX IF NOT EXISTS ar_reconciliation_runs_app_id ON billing_reconciliation_runs(app_id);
CREATE INDEX IF NOT EXISTS ar_reconciliation_runs_status ON billing_reconciliation_runs(status);
CREATE INDEX IF NOT EXISTS ar_reconciliation_runs_started_at ON billing_reconciliation_runs(started_at);

CREATE INDEX IF NOT EXISTS ar_divergences_app_id ON billing_divergences(app_id);
CREATE INDEX IF NOT EXISTS ar_divergences_run_id ON billing_divergences(run_id);
CREATE INDEX IF NOT EXISTS ar_divergences_entity_type ON billing_divergences(entity_type);
CREATE INDEX IF NOT EXISTS ar_divergences_status ON billing_divergences(status);

-- Plans
CREATE INDEX IF NOT EXISTS ar_plans_app_id ON billing_plans(app_id);
CREATE INDEX IF NOT EXISTS ar_plans_active ON billing_plans(active);

-- Coupons
CREATE INDEX IF NOT EXISTS ar_coupons_app_id ON billing_coupons(app_id);
CREATE INDEX IF NOT EXISTS ar_coupons_active ON billing_coupons(active);
CREATE INDEX IF NOT EXISTS ar_coupons_redeem_by ON billing_coupons(redeem_by);

-- Addons
CREATE INDEX IF NOT EXISTS ar_addons_app_id ON billing_addons(app_id);
CREATE INDEX IF NOT EXISTS ar_addons_active ON billing_addons(active);

-- Subscription Addons
CREATE INDEX IF NOT EXISTS ar_subscription_addons_app_id ON billing_subscription_addons(app_id);
CREATE INDEX IF NOT EXISTS ar_subscription_addons_subscription_id ON billing_subscription_addons(subscription_id);

-- Invoices
CREATE INDEX IF NOT EXISTS ar_invoices_app_id ON billing_invoices(app_id);
CREATE INDEX IF NOT EXISTS ar_invoices_customer_id ON billing_invoices(billing_customer_id);
CREATE INDEX IF NOT EXISTS ar_invoices_subscription_id ON billing_invoices(subscription_id);
CREATE INDEX IF NOT EXISTS ar_invoices_status ON billing_invoices(status);
CREATE INDEX IF NOT EXISTS ar_invoices_due_at ON billing_invoices(due_at);
CREATE INDEX IF NOT EXISTS ar_invoices_app_status_due_at ON billing_invoices(app_id, status, due_at);

-- Charges
CREATE INDEX IF NOT EXISTS ar_charges_app_id ON billing_charges(app_id);
CREATE INDEX IF NOT EXISTS ar_charges_customer_id ON billing_charges(billing_customer_id);
CREATE INDEX IF NOT EXISTS ar_charges_subscription_id ON billing_charges(subscription_id);
CREATE INDEX IF NOT EXISTS ar_charges_status ON billing_charges(status);
CREATE INDEX IF NOT EXISTS ar_charges_charge_type ON billing_charges(charge_type);
CREATE INDEX IF NOT EXISTS ar_charges_reason ON billing_charges(reason);
CREATE INDEX IF NOT EXISTS ar_charges_service_date ON billing_charges(service_date);
CREATE INDEX IF NOT EXISTS ar_charges_app_created_at ON billing_charges(app_id, created_at);

-- Refunds
CREATE INDEX IF NOT EXISTS ar_refunds_app_id ON billing_refunds(app_id);
CREATE INDEX IF NOT EXISTS ar_refunds_customer_id ON billing_refunds(billing_customer_id);
CREATE INDEX IF NOT EXISTS ar_refunds_charge_id ON billing_refunds(charge_id);
CREATE INDEX IF NOT EXISTS ar_refunds_status ON billing_refunds(status);

-- Disputes
CREATE INDEX IF NOT EXISTS ar_disputes_app_id ON billing_disputes(app_id);
CREATE INDEX IF NOT EXISTS ar_disputes_app_status ON billing_disputes(app_id, status);
CREATE INDEX IF NOT EXISTS ar_disputes_charge_id ON billing_disputes(charge_id);
CREATE INDEX IF NOT EXISTS ar_disputes_status ON billing_disputes(status);

-- Tax Rates
CREATE INDEX IF NOT EXISTS ar_tax_rates_app_jurisdiction ON billing_tax_rates(app_id, jurisdiction_code);
CREATE INDEX IF NOT EXISTS ar_tax_rates_effective_date ON billing_tax_rates(effective_date);

-- Tax Calculations
CREATE INDEX IF NOT EXISTS ar_tax_calculations_app_invoice ON billing_tax_calculations(app_id, invoice_id);
CREATE INDEX IF NOT EXISTS ar_tax_calculations_app_charge ON billing_tax_calculations(app_id, charge_id);
CREATE INDEX IF NOT EXISTS ar_tax_calculations_tax_rate ON billing_tax_calculations(tax_rate_id);

-- Discount Applications
CREATE INDEX IF NOT EXISTS ar_discount_applications_app_invoice ON billing_discount_applications(app_id, invoice_id);
CREATE INDEX IF NOT EXISTS ar_discount_applications_app_charge ON billing_discount_applications(app_id, charge_id);
CREATE INDEX IF NOT EXISTS ar_discount_applications_app_coupon ON billing_discount_applications(app_id, coupon_id);
CREATE INDEX IF NOT EXISTS ar_discount_applications_app_customer ON billing_discount_applications(app_id, customer_id);
CREATE INDEX IF NOT EXISTS ar_discount_applications_applied_at ON billing_discount_applications(applied_at);

-- Metered Usage
CREATE INDEX IF NOT EXISTS ar_metered_usage_app_customer ON billing_metered_usage(app_id, customer_id);
CREATE INDEX IF NOT EXISTS ar_metered_usage_app_subscription ON billing_metered_usage(app_id, subscription_id);
CREATE INDEX IF NOT EXISTS ar_metered_usage_period ON billing_metered_usage(period_start, period_end);
CREATE INDEX IF NOT EXISTS ar_metered_usage_billed_at ON billing_metered_usage(billed_at);

-- Dunning Config
CREATE INDEX IF NOT EXISTS ar_dunning_config_app_id ON billing_dunning_config(app_id);

-- Invoice Line Items
CREATE INDEX IF NOT EXISTS ar_invoice_line_items_app_invoice ON billing_invoice_line_items(app_id, invoice_id);
CREATE INDEX IF NOT EXISTS ar_invoice_line_items_line_item_type ON billing_invoice_line_items(line_item_type);
