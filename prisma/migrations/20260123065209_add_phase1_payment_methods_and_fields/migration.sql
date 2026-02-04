-- CreateTable
CREATE TABLE `billing_customers` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `external_customer_id` VARCHAR(255) NULL,
    `tilled_customer_id` VARCHAR(255) NOT NULL,
    `email` VARCHAR(255) NOT NULL,
    `name` VARCHAR(255) NULL,
    `default_payment_method_id` VARCHAR(255) NULL,
    `payment_method_type` VARCHAR(20) NULL,
    `metadata` JSON NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `updated_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),

    UNIQUE INDEX `billing_customers_tilled_customer_id_key`(`tilled_customer_id`),
    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_email`(`email`),
    UNIQUE INDEX `unique_app_external_customer`(`app_id`, `external_customer_id`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_subscriptions` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `billing_customer_id` INTEGER NOT NULL,
    `tilled_subscription_id` VARCHAR(255) NOT NULL,
    `plan_id` VARCHAR(100) NOT NULL,
    `plan_name` VARCHAR(255) NOT NULL,
    `price_cents` INTEGER NOT NULL,
    `status` ENUM('incomplete', 'incomplete_expired', 'trialing', 'active', 'past_due', 'canceled', 'unpaid', 'paused') NOT NULL,
    `interval_unit` ENUM('day', 'week', 'month', 'year') NOT NULL,
    `interval_count` INTEGER NOT NULL DEFAULT 1,
    `billing_cycle_anchor` TIMESTAMP(0) NULL,
    `current_period_start` TIMESTAMP(0) NOT NULL,
    `current_period_end` TIMESTAMP(0) NOT NULL,
    `cancel_at_period_end` BOOLEAN NOT NULL DEFAULT false,
    `cancel_at` TIMESTAMP(0) NULL,
    `canceled_at` TIMESTAMP(0) NULL,
    `ended_at` TIMESTAMP(0) NULL,
    `payment_method_id` VARCHAR(255) NOT NULL,
    `payment_method_type` VARCHAR(20) NOT NULL,
    `metadata` JSON NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `updated_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),

    UNIQUE INDEX `billing_subscriptions_tilled_subscription_id_key`(`tilled_subscription_id`),
    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_billing_customer_id`(`billing_customer_id`),
    INDEX `idx_status`(`status`),
    INDEX `idx_plan_id`(`plan_id`),
    INDEX `idx_current_period_end`(`current_period_end`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_payment_methods` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `billing_customer_id` INTEGER NOT NULL,
    `tilled_payment_method_id` VARCHAR(255) NOT NULL,
    `type` VARCHAR(20) NOT NULL,
    `brand` VARCHAR(50) NULL,
    `last4` VARCHAR(4) NULL,
    `exp_month` INTEGER NULL,
    `exp_year` INTEGER NULL,
    `bank_name` VARCHAR(255) NULL,
    `bank_last4` VARCHAR(4) NULL,
    `is_default` BOOLEAN NOT NULL DEFAULT false,
    `metadata` JSON NULL,
    `deleted_at` TIMESTAMP(0) NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `updated_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),

    UNIQUE INDEX `billing_payment_methods_tilled_payment_method_id_key`(`tilled_payment_method_id`),
    INDEX `idx_app_id`(`app_id`),
    INDEX `idx_billing_customer_id`(`billing_customer_id`),
    INDEX `idx_customer_default`(`billing_customer_id`, `is_default`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_webhooks` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `event_id` VARCHAR(255) NOT NULL,
    `event_type` VARCHAR(100) NOT NULL,
    `status` ENUM('received', 'processing', 'processed', 'failed') NOT NULL DEFAULT 'received',
    `error` TEXT NULL,
    `attempt_count` INTEGER NOT NULL DEFAULT 1,
    `received_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `processed_at` TIMESTAMP(0) NULL,

    UNIQUE INDEX `billing_webhooks_event_id_key`(`event_id`),
    INDEX `idx_app_status`(`app_id`, `status`),
    INDEX `idx_event_type`(`event_type`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- AddForeignKey
ALTER TABLE `billing_subscriptions` ADD CONSTRAINT `billing_subscriptions_billing_customer_id_fkey` FOREIGN KEY (`billing_customer_id`) REFERENCES `billing_customers`(`id`) ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_payment_methods` ADD CONSTRAINT `billing_payment_methods_billing_customer_id_fkey` FOREIGN KEY (`billing_customer_id`) REFERENCES `billing_customers`(`id`) ON DELETE CASCADE ON UPDATE CASCADE;
