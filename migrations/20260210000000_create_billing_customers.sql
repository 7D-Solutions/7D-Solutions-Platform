-- Create billing_customers table
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
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),

    CONSTRAINT unique_app_external_customer UNIQUE (app_id, external_customer_id)
);

-- Create indexes
CREATE INDEX IF NOT EXISTS ar_customers_app_id ON billing_customers (app_id);
CREATE INDEX IF NOT EXISTS ar_customers_email ON billing_customers (email);
CREATE INDEX IF NOT EXISTS ar_customers_delinquent_since ON billing_customers (delinquent_since);
CREATE INDEX IF NOT EXISTS ar_customers_status ON billing_customers (status);
