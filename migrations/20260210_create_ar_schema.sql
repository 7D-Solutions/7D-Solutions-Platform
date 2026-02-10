-- CreateSchema
CREATE SCHEMA IF NOT EXISTS "public";

-- CreateEnum
CREATE TYPE "billing_subscriptions_status" AS ENUM ('incomplete', 'incomplete_expired', 'trialing', 'active', 'past_due', 'canceled', 'unpaid', 'paused');

-- CreateEnum
CREATE TYPE "billing_subscriptions_interval" AS ENUM ('day', 'week', 'month', 'year');

-- CreateEnum
CREATE TYPE "billing_webhooks_status" AS ENUM ('received', 'processing', 'processed', 'failed');

-- CreateTable
CREATE TABLE "billing_customers" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "external_customer_id" VARCHAR(255),
    "tilled_customer_id" VARCHAR(255),
    "status" VARCHAR(20) NOT NULL DEFAULT 'active',
    "email" VARCHAR(255) NOT NULL,
    "name" VARCHAR(255),
    "default_payment_method_id" VARCHAR(255),
    "payment_method_type" VARCHAR(20),
    "metadata" JSONB,
    "update_source" VARCHAR(50),
    "updated_by" VARCHAR(255),
    "delinquent_since" TIMESTAMP(0),
    "grace_period_end" TIMESTAMP(0),
    "next_retry_at" TIMESTAMP(0),
    "retry_attempt_count" INTEGER NOT NULL DEFAULT 0,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "billing_customers_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_subscriptions" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "billing_customer_id" INTEGER NOT NULL,
    "tilled_subscription_id" VARCHAR(255) NOT NULL,
    "plan_id" VARCHAR(100) NOT NULL,
    "plan_name" VARCHAR(255) NOT NULL,
    "price_cents" INTEGER NOT NULL,
    "status" "billing_subscriptions_status" NOT NULL,
    "interval_unit" "billing_subscriptions_interval" NOT NULL,
    "interval_count" INTEGER NOT NULL DEFAULT 1,
    "billing_cycle_anchor" TIMESTAMP(0),
    "current_period_start" TIMESTAMP(0) NOT NULL,
    "current_period_end" TIMESTAMP(0) NOT NULL,
    "cancel_at_period_end" BOOLEAN NOT NULL DEFAULT false,
    "cancel_at" TIMESTAMP(0),
    "canceled_at" TIMESTAMP(0),
    "ended_at" TIMESTAMP(0),
    "payment_method_id" VARCHAR(255) NOT NULL,
    "payment_method_type" VARCHAR(20) NOT NULL,
    "metadata" JSONB,
    "update_source" VARCHAR(50),
    "updated_by" VARCHAR(255),
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "billing_subscriptions_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_payment_methods" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "billing_customer_id" INTEGER NOT NULL,
    "tilled_payment_method_id" VARCHAR(255) NOT NULL,
    "status" VARCHAR(20) NOT NULL DEFAULT 'active',
    "type" VARCHAR(20) NOT NULL,
    "brand" VARCHAR(50),
    "last4" VARCHAR(4),
    "exp_month" INTEGER,
    "exp_year" INTEGER,
    "bank_name" VARCHAR(255),
    "bank_last4" VARCHAR(4),
    "is_default" BOOLEAN NOT NULL DEFAULT false,
    "metadata" JSONB,
    "deleted_at" TIMESTAMP(0),
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "billing_payment_methods_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_webhooks" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "event_id" VARCHAR(255) NOT NULL,
    "event_type" VARCHAR(100) NOT NULL,
    "status" "billing_webhooks_status" NOT NULL DEFAULT 'received',
    "error" TEXT,
    "payload" JSONB,
    "attempt_count" INTEGER NOT NULL DEFAULT 1,
    "last_attempt_at" TIMESTAMP(0),
    "next_attempt_at" TIMESTAMP(0),
    "dead_at" TIMESTAMP(0),
    "error_code" VARCHAR(50),
    "received_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "processed_at" TIMESTAMP(0),

    CONSTRAINT "billing_webhooks_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_idempotency_keys" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "idempotency_key" VARCHAR(255) NOT NULL,
    "request_hash" VARCHAR(64) NOT NULL,
    "response_body" JSONB NOT NULL,
    "status_code" INTEGER NOT NULL,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "expires_at" TIMESTAMP(0) NOT NULL,

    CONSTRAINT "billing_idempotency_keys_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_events" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "event_type" VARCHAR(100) NOT NULL,
    "source" VARCHAR(20) NOT NULL,
    "entity_type" VARCHAR(50),
    "entity_id" VARCHAR(255),
    "payload" JSONB,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "billing_events_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_webhook_attempts" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "event_id" VARCHAR(255) NOT NULL,
    "attempt_number" INTEGER NOT NULL DEFAULT 1,
    "status" VARCHAR(20) NOT NULL,
    "next_attempt_at" TIMESTAMP(0),
    "error_code" VARCHAR(50),
    "error_message" TEXT,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL,

    CONSTRAINT "billing_webhook_attempts_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_reconciliation_runs" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "status" VARCHAR(20) NOT NULL,
    "started_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "finished_at" TIMESTAMP(0),
    "stats" JSONB,
    "error_message" TEXT,

    CONSTRAINT "billing_reconciliation_runs_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_divergences" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "run_id" INTEGER NOT NULL,
    "entity_type" VARCHAR(50) NOT NULL,
    "entity_key" VARCHAR(255) NOT NULL,
    "divergence_type" VARCHAR(50) NOT NULL,
    "local_snapshot" JSONB,
    "remote_snapshot" JSONB,
    "status" VARCHAR(20) NOT NULL DEFAULT 'open',
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "resolved_at" TIMESTAMP(0),

    CONSTRAINT "billing_divergences_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_plans" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "plan_id" VARCHAR(100) NOT NULL,
    "name" VARCHAR(255) NOT NULL,
    "interval_unit" VARCHAR(20) NOT NULL,
    "interval_count" INTEGER NOT NULL DEFAULT 1,
    "price_cents" INTEGER NOT NULL,
    "currency" VARCHAR(3) NOT NULL DEFAULT 'usd',
    "features" JSONB,
    "active" BOOLEAN NOT NULL DEFAULT true,
    "version_tag" VARCHAR(50),
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL,

    CONSTRAINT "billing_plans_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_coupons" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "code" VARCHAR(100) NOT NULL,
    "coupon_type" VARCHAR(20) NOT NULL,
    "value" INTEGER NOT NULL,
    "currency" VARCHAR(3),
    "duration" VARCHAR(20) NOT NULL,
    "duration_months" INTEGER,
    "max_redemptions" INTEGER,
    "redeem_by" TIMESTAMP(0),
    "active" BOOLEAN NOT NULL DEFAULT true,
    "metadata" JSONB,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL,
    "product_categories" JSONB,
    "customer_segments" JSONB,
    "min_quantity" INTEGER,
    "max_discount_amount_cents" INTEGER,
    "seasonal_start_date" TIMESTAMP(0),
    "seasonal_end_date" TIMESTAMP(0),
    "volume_tiers" JSONB,
    "referral_tier" VARCHAR(50),
    "contract_term_months" INTEGER,
    "stackable" BOOLEAN DEFAULT false,
    "priority" INTEGER DEFAULT 0,

    CONSTRAINT "billing_coupons_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_addons" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "addon_id" VARCHAR(100) NOT NULL,
    "name" VARCHAR(255) NOT NULL,
    "price_cents" INTEGER NOT NULL,
    "currency" VARCHAR(3) NOT NULL DEFAULT 'usd',
    "features" JSONB,
    "active" BOOLEAN NOT NULL DEFAULT true,
    "metadata" JSONB,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL,

    CONSTRAINT "billing_addons_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_subscription_addons" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "subscription_id" INTEGER NOT NULL,
    "addon_id" INTEGER NOT NULL,
    "quantity" INTEGER NOT NULL DEFAULT 1,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL,

    CONSTRAINT "billing_subscription_addons_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_invoices" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "tilled_invoice_id" VARCHAR(255) NOT NULL,
    "billing_customer_id" INTEGER NOT NULL,
    "subscription_id" INTEGER,
    "status" VARCHAR(20) NOT NULL,
    "amount_cents" INTEGER NOT NULL,
    "currency" VARCHAR(3) NOT NULL DEFAULT 'usd',
    "due_at" TIMESTAMP(0),
    "paid_at" TIMESTAMP(0),
    "hosted_url" VARCHAR(500),
    "metadata" JSONB,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL,
    "billing_period_start" TIMESTAMP(0),
    "billing_period_end" TIMESTAMP(0),
    "line_item_details" JSONB,
    "compliance_codes" JSONB,

    CONSTRAINT "billing_invoices_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_charges" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "tilled_charge_id" VARCHAR(255),
    "invoice_id" INTEGER,
    "billing_customer_id" INTEGER NOT NULL,
    "subscription_id" INTEGER,
    "status" VARCHAR(20) NOT NULL,
    "amount_cents" INTEGER NOT NULL,
    "currency" VARCHAR(3) NOT NULL DEFAULT 'usd',
    "charge_type" VARCHAR(50) NOT NULL DEFAULT 'one_time',
    "reason" VARCHAR(100),
    "reference_id" VARCHAR(255),
    "service_date" TIMESTAMP(0),
    "note" TEXT,
    "metadata" JSONB,
    "failure_code" VARCHAR(50),
    "failure_message" TEXT,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL,
    "product_type" VARCHAR(50),
    "quantity" INTEGER DEFAULT 1,
    "service_frequency" VARCHAR(20),
    "weight_amount" DECIMAL(10,2),
    "location_reference" VARCHAR(500),

    CONSTRAINT "billing_charges_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_refunds" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "billing_customer_id" INTEGER NOT NULL,
    "charge_id" INTEGER NOT NULL,
    "tilled_refund_id" VARCHAR(255),
    "tilled_charge_id" VARCHAR(255),
    "status" VARCHAR(20) NOT NULL,
    "amount_cents" INTEGER NOT NULL,
    "currency" VARCHAR(3) NOT NULL DEFAULT 'usd',
    "reason" VARCHAR(100),
    "reference_id" VARCHAR(255) NOT NULL,
    "note" TEXT,
    "metadata" JSONB,
    "failure_code" VARCHAR(50),
    "failure_message" TEXT,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "billing_refunds_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_disputes" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "tilled_dispute_id" VARCHAR(255) NOT NULL,
    "tilled_charge_id" VARCHAR(255),
    "charge_id" INTEGER,
    "status" VARCHAR(30) NOT NULL,
    "amount_cents" INTEGER,
    "currency" VARCHAR(3),
    "reason" VARCHAR(255),
    "reason_code" VARCHAR(50),
    "evidence_due_by" TIMESTAMP(0),
    "opened_at" TIMESTAMP(0),
    "closed_at" TIMESTAMP(0),
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "billing_disputes_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_tax_rates" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "jurisdiction_code" VARCHAR(20) NOT NULL,
    "tax_type" VARCHAR(50) NOT NULL,
    "rate" DECIMAL(5,4) NOT NULL,
    "effective_date" TIMESTAMP(0) NOT NULL,
    "expiration_date" TIMESTAMP(0),
    "description" VARCHAR(255),
    "metadata" JSONB,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL,

    CONSTRAINT "billing_tax_rates_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_tax_calculations" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "invoice_id" INTEGER,
    "charge_id" INTEGER,
    "tax_rate_id" INTEGER NOT NULL,
    "taxable_amount_cents" INTEGER NOT NULL,
    "tax_amount_cents" INTEGER NOT NULL,
    "jurisdiction_code" VARCHAR(20) NOT NULL,
    "tax_type" VARCHAR(50) NOT NULL,
    "rate_applied" DECIMAL(5,4) NOT NULL,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "billing_tax_calculations_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_discount_applications" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "invoice_id" INTEGER,
    "charge_id" INTEGER,
    "coupon_id" INTEGER,
    "customer_id" INTEGER,
    "discount_type" VARCHAR(50) NOT NULL,
    "discount_amount_cents" INTEGER NOT NULL,
    "description" VARCHAR(255) NOT NULL,
    "quantity" INTEGER,
    "category" VARCHAR(50),
    "product_types" JSONB,
    "metadata" JSONB,
    "applied_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "created_by" VARCHAR(255),
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "billing_discount_applications_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_metered_usage" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "customer_id" INTEGER NOT NULL,
    "subscription_id" INTEGER,
    "metric_name" VARCHAR(100) NOT NULL,
    "quantity" DECIMAL(10,2) NOT NULL,
    "unit_price_cents" INTEGER NOT NULL,
    "period_start" TIMESTAMP(0) NOT NULL,
    "period_end" TIMESTAMP(0) NOT NULL,
    "recorded_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "billed_at" TIMESTAMP(0),

    CONSTRAINT "billing_metered_usage_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_dunning_config" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50),
    "grace_period_days" INTEGER NOT NULL DEFAULT 3,
    "retry_schedule_days" JSONB NOT NULL,
    "max_retry_attempts" INTEGER NOT NULL DEFAULT 3,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(0) NOT NULL,

    CONSTRAINT "billing_dunning_config_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "billing_invoice_line_items" (
    "id" SERIAL NOT NULL,
    "app_id" VARCHAR(50) NOT NULL,
    "invoice_id" INTEGER NOT NULL,
    "line_item_type" VARCHAR(50) NOT NULL,
    "description" VARCHAR(500) NOT NULL,
    "quantity" DECIMAL(10,2) NOT NULL,
    "unit_price_cents" INTEGER NOT NULL,
    "amount_cents" INTEGER NOT NULL,
    "metadata" JSONB,
    "created_at" TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "billing_invoice_line_items_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE UNIQUE INDEX "billing_customers_tilled_customer_id_key" ON "billing_customers"("tilled_customer_id");

-- CreateIndex
CREATE INDEX "ar_customers_app_id" ON "billing_customers"("app_id");

-- CreateIndex
CREATE INDEX "ar_customers_email" ON "billing_customers"("email");

-- CreateIndex
CREATE INDEX "ar_customers_delinquent_since" ON "billing_customers"("delinquent_since");

-- CreateIndex
CREATE INDEX "ar_customers_next_retry_at" ON "billing_customers"("next_retry_at");

-- CreateIndex
CREATE UNIQUE INDEX "unique_app_external_customer" ON "billing_customers"("app_id", "external_customer_id");

-- CreateIndex
CREATE UNIQUE INDEX "billing_subscriptions_tilled_subscription_id_key" ON "billing_subscriptions"("tilled_subscription_id");

-- CreateIndex
CREATE INDEX "ar_subscriptions_app_id" ON "billing_subscriptions"("app_id");

-- CreateIndex
CREATE INDEX "ar_subscriptions_customer_id" ON "billing_subscriptions"("billing_customer_id");

-- CreateIndex
CREATE INDEX "ar_subscriptions_status" ON "billing_subscriptions"("status");

-- CreateIndex
CREATE INDEX "ar_subscriptions_plan_id" ON "billing_subscriptions"("plan_id");

-- CreateIndex
CREATE INDEX "ar_subscriptions_current_period_end" ON "billing_subscriptions"("current_period_end");

-- CreateIndex
CREATE INDEX "ar_subscriptions_app_status_period_end" ON "billing_subscriptions"("app_id", "status", "current_period_end");

-- CreateIndex
CREATE UNIQUE INDEX "billing_payment_methods_tilled_payment_method_id_key" ON "billing_payment_methods"("tilled_payment_method_id");

-- CreateIndex
CREATE INDEX "ar_payment_methods_app_id" ON "billing_payment_methods"("app_id");

-- CreateIndex
CREATE INDEX "ar_payment_methods_customer_id" ON "billing_payment_methods"("billing_customer_id");

-- CreateIndex
CREATE INDEX "ar_payment_methods_customer_default" ON "billing_payment_methods"("billing_customer_id", "is_default");

-- CreateIndex
CREATE INDEX "ar_webhooks_app_status" ON "billing_webhooks"("app_id", "status");

-- CreateIndex
CREATE INDEX "ar_webhooks_event_type" ON "billing_webhooks"("event_type");

-- CreateIndex
CREATE INDEX "ar_webhooks_next_attempt_at" ON "billing_webhooks"("next_attempt_at");

-- CreateIndex
CREATE UNIQUE INDEX "event_id_app_id" ON "billing_webhooks"("event_id", "app_id");

-- CreateIndex
CREATE INDEX "ar_idempotency_keys_app_id" ON "billing_idempotency_keys"("app_id");

-- CreateIndex
CREATE INDEX "ar_idempotency_keys_expires_at" ON "billing_idempotency_keys"("expires_at");

-- CreateIndex
CREATE UNIQUE INDEX "unique_app_idempotency_key" ON "billing_idempotency_keys"("app_id", "idempotency_key");

-- CreateIndex
CREATE INDEX "ar_events_app_id" ON "billing_events"("app_id");

-- CreateIndex
CREATE INDEX "ar_events_event_type" ON "billing_events"("event_type");

-- CreateIndex
CREATE INDEX "ar_events_source" ON "billing_events"("source");

-- CreateIndex
CREATE INDEX "ar_events_created_at" ON "billing_events"("created_at");

-- CreateIndex
CREATE INDEX "ar_webhook_attempts_app_id" ON "billing_webhook_attempts"("app_id");

-- CreateIndex
CREATE INDEX "ar_webhook_attempts_event_id" ON "billing_webhook_attempts"("event_id");

-- CreateIndex
CREATE INDEX "ar_webhook_attempts_status" ON "billing_webhook_attempts"("status");

-- CreateIndex
CREATE INDEX "ar_webhook_attempts_next_attempt_at" ON "billing_webhook_attempts"("next_attempt_at");

-- CreateIndex
CREATE INDEX "ar_reconciliation_runs_app_id" ON "billing_reconciliation_runs"("app_id");

-- CreateIndex
CREATE INDEX "ar_reconciliation_runs_status" ON "billing_reconciliation_runs"("status");

-- CreateIndex
CREATE INDEX "ar_reconciliation_runs_started_at" ON "billing_reconciliation_runs"("started_at");

-- CreateIndex
CREATE INDEX "ar_divergences_app_id" ON "billing_divergences"("app_id");

-- CreateIndex
CREATE INDEX "ar_divergences_run_id" ON "billing_divergences"("run_id");

-- CreateIndex
CREATE INDEX "ar_divergences_entity_type" ON "billing_divergences"("entity_type");

-- CreateIndex
CREATE INDEX "ar_divergences_status" ON "billing_divergences"("status");

-- CreateIndex
CREATE INDEX "ar_plans_app_id" ON "billing_plans"("app_id");

-- CreateIndex
CREATE INDEX "ar_plans_active" ON "billing_plans"("active");

-- CreateIndex
CREATE UNIQUE INDEX "unique_app_plan_id" ON "billing_plans"("app_id", "plan_id");

-- CreateIndex
CREATE INDEX "ar_coupons_app_id" ON "billing_coupons"("app_id");

-- CreateIndex
CREATE INDEX "ar_coupons_active" ON "billing_coupons"("active");

-- CreateIndex
CREATE INDEX "ar_coupons_redeem_by" ON "billing_coupons"("redeem_by");

-- CreateIndex
CREATE UNIQUE INDEX "unique_app_code" ON "billing_coupons"("app_id", "code");

-- CreateIndex
CREATE INDEX "ar_addons_app_id" ON "billing_addons"("app_id");

-- CreateIndex
CREATE INDEX "ar_addons_active" ON "billing_addons"("active");

-- CreateIndex
CREATE UNIQUE INDEX "unique_app_addon_id" ON "billing_addons"("app_id", "addon_id");

-- CreateIndex
CREATE INDEX "ar_subscription_addons_app_id" ON "billing_subscription_addons"("app_id");

-- CreateIndex
CREATE INDEX "ar_subscription_addons_subscription_id" ON "billing_subscription_addons"("subscription_id");

-- CreateIndex
CREATE UNIQUE INDEX "unique_subscription_addon" ON "billing_subscription_addons"("subscription_id", "addon_id");

-- CreateIndex
CREATE UNIQUE INDEX "billing_invoices_tilled_invoice_id_key" ON "billing_invoices"("tilled_invoice_id");

-- CreateIndex
CREATE INDEX "ar_invoices_app_id" ON "billing_invoices"("app_id");

-- CreateIndex
CREATE INDEX "ar_invoices_customer_id" ON "billing_invoices"("billing_customer_id");

-- CreateIndex
CREATE INDEX "ar_invoices_subscription_id" ON "billing_invoices"("subscription_id");

-- CreateIndex
CREATE INDEX "ar_invoices_status" ON "billing_invoices"("status");

-- CreateIndex
CREATE INDEX "ar_invoices_due_at" ON "billing_invoices"("due_at");

-- CreateIndex
CREATE INDEX "ar_invoices_app_status_due_at" ON "billing_invoices"("app_id", "status", "due_at");

-- CreateIndex
CREATE UNIQUE INDEX "billing_charges_tilled_charge_id_key" ON "billing_charges"("tilled_charge_id");

-- CreateIndex
CREATE INDEX "ar_charges_app_id" ON "billing_charges"("app_id");

-- CreateIndex
CREATE INDEX "ar_charges_customer_id" ON "billing_charges"("billing_customer_id");

-- CreateIndex
CREATE INDEX "ar_charges_subscription_id" ON "billing_charges"("subscription_id");

-- CreateIndex
CREATE INDEX "ar_charges_status" ON "billing_charges"("status");

-- CreateIndex
CREATE INDEX "ar_charges_charge_type" ON "billing_charges"("charge_type");

-- CreateIndex
CREATE INDEX "ar_charges_reason" ON "billing_charges"("reason");

-- CreateIndex
CREATE INDEX "ar_charges_service_date" ON "billing_charges"("service_date");

-- CreateIndex
CREATE INDEX "ar_charges_app_created_at" ON "billing_charges"("app_id", "created_at");

-- CreateIndex
CREATE UNIQUE INDEX "unique_app_reference_id" ON "billing_charges"("app_id", "reference_id");

-- CreateIndex
CREATE INDEX "ar_refunds_app_id" ON "billing_refunds"("app_id");

-- CreateIndex
CREATE INDEX "ar_refunds_customer_id" ON "billing_refunds"("billing_customer_id");

-- CreateIndex
CREATE INDEX "ar_refunds_charge_id" ON "billing_refunds"("charge_id");

-- CreateIndex
CREATE INDEX "ar_refunds_status" ON "billing_refunds"("status");

-- CreateIndex
CREATE UNIQUE INDEX "unique_refund_app_reference_id" ON "billing_refunds"("app_id", "reference_id");

-- CreateIndex
CREATE UNIQUE INDEX "tilled_refund_id_app_id" ON "billing_refunds"("tilled_refund_id", "app_id");

-- CreateIndex
CREATE INDEX "ar_disputes_app_id" ON "billing_disputes"("app_id");

-- CreateIndex
CREATE INDEX "ar_disputes_app_status" ON "billing_disputes"("app_id", "status");

-- CreateIndex
CREATE INDEX "ar_disputes_charge_id" ON "billing_disputes"("charge_id");

-- CreateIndex
CREATE INDEX "ar_disputes_status" ON "billing_disputes"("status");

-- CreateIndex
CREATE UNIQUE INDEX "tilled_dispute_id_app_id" ON "billing_disputes"("tilled_dispute_id", "app_id");

-- CreateIndex
CREATE INDEX "ar_tax_rates_app_jurisdiction" ON "billing_tax_rates"("app_id", "jurisdiction_code");

-- CreateIndex
CREATE INDEX "ar_tax_rates_effective_date" ON "billing_tax_rates"("effective_date");

-- CreateIndex
CREATE UNIQUE INDEX "unique_tax_rate" ON "billing_tax_rates"("app_id", "jurisdiction_code", "tax_type", "effective_date");

-- CreateIndex
CREATE INDEX "ar_tax_calculations_app_invoice" ON "billing_tax_calculations"("app_id", "invoice_id");

-- CreateIndex
CREATE INDEX "ar_tax_calculations_app_charge" ON "billing_tax_calculations"("app_id", "charge_id");

-- CreateIndex
CREATE INDEX "ar_tax_calculations_tax_rate" ON "billing_tax_calculations"("tax_rate_id");

-- CreateIndex
CREATE INDEX "ar_discount_applications_app_invoice" ON "billing_discount_applications"("app_id", "invoice_id");

-- CreateIndex
CREATE INDEX "ar_discount_applications_app_charge" ON "billing_discount_applications"("app_id", "charge_id");

-- CreateIndex
CREATE INDEX "ar_discount_applications_app_coupon" ON "billing_discount_applications"("app_id", "coupon_id");

-- CreateIndex
CREATE INDEX "ar_discount_applications_app_customer" ON "billing_discount_applications"("app_id", "customer_id");

-- CreateIndex
CREATE INDEX "ar_discount_applications_applied_at" ON "billing_discount_applications"("applied_at");

-- CreateIndex
CREATE INDEX "ar_metered_usage_app_customer" ON "billing_metered_usage"("app_id", "customer_id");

-- CreateIndex
CREATE INDEX "ar_metered_usage_app_subscription" ON "billing_metered_usage"("app_id", "subscription_id");

-- CreateIndex
CREATE INDEX "ar_metered_usage_period" ON "billing_metered_usage"("period_start", "period_end");

-- CreateIndex
CREATE INDEX "ar_metered_usage_billed_at" ON "billing_metered_usage"("billed_at");

-- CreateIndex
CREATE INDEX "ar_dunning_config_app_id" ON "billing_dunning_config"("app_id");

-- CreateIndex
CREATE UNIQUE INDEX "unique_app_dunning_config" ON "billing_dunning_config"("app_id");

-- CreateIndex
CREATE INDEX "ar_invoice_line_items_app_invoice" ON "billing_invoice_line_items"("app_id", "invoice_id");

-- CreateIndex
CREATE INDEX "ar_invoice_line_items_line_item_type" ON "billing_invoice_line_items"("line_item_type");

-- AddForeignKey
ALTER TABLE "billing_subscriptions" ADD CONSTRAINT "billing_subscriptions_billing_customer_id_fkey" FOREIGN KEY ("billing_customer_id") REFERENCES "billing_customers"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_payment_methods" ADD CONSTRAINT "billing_payment_methods_billing_customer_id_fkey" FOREIGN KEY ("billing_customer_id") REFERENCES "billing_customers"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_divergences" ADD CONSTRAINT "billing_divergences_run_id_fkey" FOREIGN KEY ("run_id") REFERENCES "billing_reconciliation_runs"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_subscription_addons" ADD CONSTRAINT "billing_subscription_addons_subscription_id_fkey" FOREIGN KEY ("subscription_id") REFERENCES "billing_subscriptions"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_subscription_addons" ADD CONSTRAINT "billing_subscription_addons_addon_id_fkey" FOREIGN KEY ("addon_id") REFERENCES "billing_addons"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_invoices" ADD CONSTRAINT "billing_invoices_billing_customer_id_fkey" FOREIGN KEY ("billing_customer_id") REFERENCES "billing_customers"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_invoices" ADD CONSTRAINT "billing_invoices_subscription_id_fkey" FOREIGN KEY ("subscription_id") REFERENCES "billing_subscriptions"("id") ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_charges" ADD CONSTRAINT "billing_charges_invoice_id_fkey" FOREIGN KEY ("invoice_id") REFERENCES "billing_invoices"("id") ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_charges" ADD CONSTRAINT "billing_charges_billing_customer_id_fkey" FOREIGN KEY ("billing_customer_id") REFERENCES "billing_customers"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_charges" ADD CONSTRAINT "billing_charges_subscription_id_fkey" FOREIGN KEY ("subscription_id") REFERENCES "billing_subscriptions"("id") ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_refunds" ADD CONSTRAINT "billing_refunds_charge_id_fkey" FOREIGN KEY ("charge_id") REFERENCES "billing_charges"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_refunds" ADD CONSTRAINT "billing_refunds_billing_customer_id_fkey" FOREIGN KEY ("billing_customer_id") REFERENCES "billing_customers"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_disputes" ADD CONSTRAINT "billing_disputes_charge_id_fkey" FOREIGN KEY ("charge_id") REFERENCES "billing_charges"("id") ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_tax_calculations" ADD CONSTRAINT "billing_tax_calculations_invoice_id_fkey" FOREIGN KEY ("invoice_id") REFERENCES "billing_invoices"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_tax_calculations" ADD CONSTRAINT "billing_tax_calculations_charge_id_fkey" FOREIGN KEY ("charge_id") REFERENCES "billing_charges"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_tax_calculations" ADD CONSTRAINT "billing_tax_calculations_tax_rate_id_fkey" FOREIGN KEY ("tax_rate_id") REFERENCES "billing_tax_rates"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_discount_applications" ADD CONSTRAINT "billing_discount_applications_invoice_id_fkey" FOREIGN KEY ("invoice_id") REFERENCES "billing_invoices"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_discount_applications" ADD CONSTRAINT "billing_discount_applications_charge_id_fkey" FOREIGN KEY ("charge_id") REFERENCES "billing_charges"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_discount_applications" ADD CONSTRAINT "billing_discount_applications_coupon_id_fkey" FOREIGN KEY ("coupon_id") REFERENCES "billing_coupons"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_discount_applications" ADD CONSTRAINT "billing_discount_applications_customer_id_fkey" FOREIGN KEY ("customer_id") REFERENCES "billing_customers"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_metered_usage" ADD CONSTRAINT "billing_metered_usage_customer_id_fkey" FOREIGN KEY ("customer_id") REFERENCES "billing_customers"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_metered_usage" ADD CONSTRAINT "billing_metered_usage_subscription_id_fkey" FOREIGN KEY ("subscription_id") REFERENCES "billing_subscriptions"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "billing_invoice_line_items" ADD CONSTRAINT "billing_invoice_line_items_invoice_id_fkey" FOREIGN KEY ("invoice_id") REFERENCES "billing_invoices"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

