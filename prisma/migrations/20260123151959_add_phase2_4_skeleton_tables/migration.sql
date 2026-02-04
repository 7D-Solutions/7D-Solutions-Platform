-- AlterTable
ALTER TABLE `billing_customers` ADD COLUMN `delinquent_since` TIMESTAMP(0) NULL,
    ADD COLUMN `grace_period_end` TIMESTAMP(0) NULL,
    ADD COLUMN `update_source` VARCHAR(50) NULL,
    ADD COLUMN `updated_by` VARCHAR(255) NULL;

-- AlterTable
ALTER TABLE `billing_subscriptions` ADD COLUMN `update_source` VARCHAR(50) NULL,
    ADD COLUMN `updated_by` VARCHAR(255) NULL;

-- AlterTable
ALTER TABLE `billing_webhooks` ADD COLUMN `dead_at` TIMESTAMP(0) NULL,
    ADD COLUMN `error_code` VARCHAR(50) NULL,
    ADD COLUMN `last_attempt_at` TIMESTAMP(0) NULL,
    ADD COLUMN `next_attempt_at` TIMESTAMP(0) NULL;

-- CreateTable
CREATE TABLE `billing_idempotency_keys` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `idempotency_key` VARCHAR(255) NOT NULL,
    `request_hash` VARCHAR(64) NOT NULL,
    `response_body` JSON NOT NULL,
    `status_code` INTEGER NOT NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `expires_at` TIMESTAMP(0) NOT NULL,

    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_expires_at`(`expires_at`),
    UNIQUE INDEX `unique_app_idempotency_key`(`app_id`, `idempotency_key`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_events` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `event_type` VARCHAR(100) NOT NULL,
    `source` VARCHAR(20) NOT NULL,
    `entity_type` VARCHAR(50) NULL,
    `entity_id` VARCHAR(255) NULL,
    `payload` JSON NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),

    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_event_type`(`event_type`),
    INDEX `idx_source`(`source`),
    INDEX `idx_created_at`(`created_at`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_webhook_attempts` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `event_id` VARCHAR(255) NOT NULL,
    `attempt_number` INTEGER NOT NULL DEFAULT 1,
    `status` VARCHAR(20) NOT NULL,
    `next_attempt_at` TIMESTAMP(0) NULL,
    `error_code` VARCHAR(50) NULL,
    `error_message` TEXT NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `updated_at` TIMESTAMP(0) NOT NULL,

    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_event_id`(`event_id`),
    INDEX `idx_status`(`status`),
    INDEX `idx_next_attempt_at`(`next_attempt_at`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_reconciliation_runs` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `status` VARCHAR(20) NOT NULL,
    `started_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `finished_at` TIMESTAMP(0) NULL,
    `stats` JSON NULL,
    `error_message` TEXT NULL,

    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_status`(`status`),
    INDEX `idx_started_at`(`started_at`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_divergences` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `run_id` INTEGER NOT NULL,
    `entity_type` VARCHAR(50) NOT NULL,
    `entity_key` VARCHAR(255) NOT NULL,
    `divergence_type` VARCHAR(50) NOT NULL,
    `local_snapshot` JSON NULL,
    `remote_snapshot` JSON NULL,
    `status` VARCHAR(20) NOT NULL DEFAULT 'open',
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `resolved_at` TIMESTAMP(0) NULL,

    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_run_id`(`run_id`),
    INDEX `idx_entity_type`(`entity_type`),
    INDEX `idx_status`(`status`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_plans` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `plan_id` VARCHAR(100) NOT NULL,
    `name` VARCHAR(255) NOT NULL,
    `interval_unit` VARCHAR(20) NOT NULL,
    `interval_count` INTEGER NOT NULL DEFAULT 1,
    `price_cents` INTEGER NOT NULL,
    `currency` VARCHAR(3) NOT NULL DEFAULT 'usd',
    `features` JSON NULL,
    `active` BOOLEAN NOT NULL DEFAULT true,
    `version_tag` VARCHAR(50) NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `updated_at` TIMESTAMP(0) NOT NULL,

    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_active`(`active`),
    UNIQUE INDEX `unique_app_plan_id`(`app_id`, `plan_id`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_coupons` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `code` VARCHAR(100) NOT NULL,
    `coupon_type` VARCHAR(20) NOT NULL,
    `value` INTEGER NOT NULL,
    `currency` VARCHAR(3) NULL,
    `duration` VARCHAR(20) NOT NULL,
    `duration_months` INTEGER NULL,
    `max_redemptions` INTEGER NULL,
    `redeem_by` TIMESTAMP(0) NULL,
    `active` BOOLEAN NOT NULL DEFAULT true,
    `metadata` JSON NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `updated_at` TIMESTAMP(0) NOT NULL,

    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_active`(`active`),
    INDEX `idx_redeem_by`(`redeem_by`),
    UNIQUE INDEX `unique_app_code`(`app_id`, `code`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_addons` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `addon_id` VARCHAR(100) NOT NULL,
    `name` VARCHAR(255) NOT NULL,
    `price_cents` INTEGER NOT NULL,
    `currency` VARCHAR(3) NOT NULL DEFAULT 'usd',
    `features` JSON NULL,
    `active` BOOLEAN NOT NULL DEFAULT true,
    `metadata` JSON NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `updated_at` TIMESTAMP(0) NOT NULL,

    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_active`(`active`),
    UNIQUE INDEX `unique_app_addon_id`(`app_id`, `addon_id`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_subscription_addons` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `subscription_id` INTEGER NOT NULL,
    `addon_id` INTEGER NOT NULL,
    `quantity` INTEGER NOT NULL DEFAULT 1,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `updated_at` TIMESTAMP(0) NOT NULL,

    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_subscription_id`(`subscription_id`),
    UNIQUE INDEX `unique_subscription_addon`(`subscription_id`, `addon_id`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_invoices` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `tilled_invoice_id` VARCHAR(255) NOT NULL,
    `billing_customer_id` INTEGER NOT NULL,
    `subscription_id` INTEGER NULL,
    `status` VARCHAR(20) NOT NULL,
    `amount_cents` INTEGER NOT NULL,
    `currency` VARCHAR(3) NOT NULL DEFAULT 'usd',
    `due_at` TIMESTAMP(0) NULL,
    `paid_at` TIMESTAMP(0) NULL,
    `hosted_url` VARCHAR(500) NULL,
    `metadata` JSON NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `updated_at` TIMESTAMP(0) NOT NULL,

    UNIQUE INDEX `billing_invoices_tilled_invoice_id_key`(`tilled_invoice_id`),
    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_billing_customer_id`(`billing_customer_id`),
    INDEX `idx_subscription_id`(`subscription_id`),
    INDEX `idx_status`(`status`),
    INDEX `idx_due_at`(`due_at`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_charges` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `tilled_charge_id` VARCHAR(255) NOT NULL,
    `invoice_id` INTEGER NULL,
    `billing_customer_id` INTEGER NOT NULL,
    `subscription_id` INTEGER NULL,
    `status` VARCHAR(20) NOT NULL,
    `amount_cents` INTEGER NOT NULL,
    `currency` VARCHAR(3) NOT NULL DEFAULT 'usd',
    `failure_code` VARCHAR(50) NULL,
    `failure_message` TEXT NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),

    UNIQUE INDEX `billing_charges_tilled_charge_id_key`(`tilled_charge_id`),
    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_billing_customer_id`(`billing_customer_id`),
    INDEX `idx_subscription_id`(`subscription_id`),
    INDEX `idx_status`(`status`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_refunds` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `tilled_refund_id` VARCHAR(255) NOT NULL,
    `charge_id` INTEGER NOT NULL,
    `status` VARCHAR(20) NOT NULL,
    `amount_cents` INTEGER NOT NULL,
    `currency` VARCHAR(3) NOT NULL DEFAULT 'usd',
    `reason` VARCHAR(255) NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),

    UNIQUE INDEX `billing_refunds_tilled_refund_id_key`(`tilled_refund_id`),
    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_charge_id`(`charge_id`),
    INDEX `idx_status`(`status`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_disputes` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `tilled_dispute_id` VARCHAR(255) NOT NULL,
    `charge_id` INTEGER NOT NULL,
    `status` VARCHAR(20) NOT NULL,
    `amount_cents` INTEGER NOT NULL,
    `currency` VARCHAR(3) NOT NULL DEFAULT 'usd',
    `reason` VARCHAR(255) NULL,
    `opened_at` TIMESTAMP(0) NULL,
    `closed_at` TIMESTAMP(0) NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),

    UNIQUE INDEX `billing_disputes_tilled_dispute_id_key`(`tilled_dispute_id`),
    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_charge_id`(`charge_id`),
    INDEX `idx_status`(`status`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateIndex
CREATE INDEX `idx_delinquent_since` ON `billing_customers`(`delinquent_since`);

-- CreateIndex
CREATE INDEX `idx_next_attempt_at` ON `billing_webhooks`(`next_attempt_at`);

-- AddForeignKey
ALTER TABLE `billing_divergences` ADD CONSTRAINT `billing_divergences_run_id_fkey` FOREIGN KEY (`run_id`) REFERENCES `billing_reconciliation_runs`(`id`) ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_subscription_addons` ADD CONSTRAINT `billing_subscription_addons_subscription_id_fkey` FOREIGN KEY (`subscription_id`) REFERENCES `billing_subscriptions`(`id`) ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_subscription_addons` ADD CONSTRAINT `billing_subscription_addons_addon_id_fkey` FOREIGN KEY (`addon_id`) REFERENCES `billing_addons`(`id`) ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_invoices` ADD CONSTRAINT `billing_invoices_billing_customer_id_fkey` FOREIGN KEY (`billing_customer_id`) REFERENCES `billing_customers`(`id`) ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_invoices` ADD CONSTRAINT `billing_invoices_subscription_id_fkey` FOREIGN KEY (`subscription_id`) REFERENCES `billing_subscriptions`(`id`) ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_charges` ADD CONSTRAINT `billing_charges_invoice_id_fkey` FOREIGN KEY (`invoice_id`) REFERENCES `billing_invoices`(`id`) ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_charges` ADD CONSTRAINT `billing_charges_billing_customer_id_fkey` FOREIGN KEY (`billing_customer_id`) REFERENCES `billing_customers`(`id`) ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_charges` ADD CONSTRAINT `billing_charges_subscription_id_fkey` FOREIGN KEY (`subscription_id`) REFERENCES `billing_subscriptions`(`id`) ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_refunds` ADD CONSTRAINT `billing_refunds_charge_id_fkey` FOREIGN KEY (`charge_id`) REFERENCES `billing_charges`(`id`) ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_disputes` ADD CONSTRAINT `billing_disputes_charge_id_fkey` FOREIGN KEY (`charge_id`) REFERENCES `billing_charges`(`id`) ON DELETE CASCADE ON UPDATE CASCADE;
