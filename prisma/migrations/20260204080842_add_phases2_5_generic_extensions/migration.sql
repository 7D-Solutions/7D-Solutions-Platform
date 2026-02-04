-- AlterTable
ALTER TABLE `billing_charges` ADD COLUMN `location_reference` VARCHAR(500) NULL,
    ADD COLUMN `product_type` VARCHAR(50) NULL,
    ADD COLUMN `quantity` INTEGER NULL DEFAULT 1,
    ADD COLUMN `service_frequency` VARCHAR(20) NULL,
    ADD COLUMN `weight_amount` DECIMAL(10, 2) NULL;

-- AlterTable
ALTER TABLE `billing_coupons` ADD COLUMN `contract_term_months` INTEGER NULL,
    ADD COLUMN `customer_segments` JSON NULL,
    ADD COLUMN `max_discount_amount_cents` INTEGER NULL,
    ADD COLUMN `min_quantity` INTEGER NULL,
    ADD COLUMN `priority` INTEGER NULL DEFAULT 0,
    ADD COLUMN `product_categories` JSON NULL,
    ADD COLUMN `referral_tier` VARCHAR(50) NULL,
    ADD COLUMN `seasonal_end_date` TIMESTAMP(0) NULL,
    ADD COLUMN `seasonal_start_date` TIMESTAMP(0) NULL,
    ADD COLUMN `stackable` BOOLEAN NULL DEFAULT false,
    ADD COLUMN `volume_tiers` JSON NULL;

-- AlterTable
ALTER TABLE `billing_invoices` ADD COLUMN `billing_period_end` TIMESTAMP(0) NULL,
    ADD COLUMN `billing_period_start` TIMESTAMP(0) NULL,
    ADD COLUMN `compliance_codes` JSON NULL,
    ADD COLUMN `line_item_details` JSON NULL;

-- CreateTable
CREATE TABLE `billing_tax_rates` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `jurisdiction_code` VARCHAR(20) NOT NULL,
    `tax_type` VARCHAR(50) NOT NULL,
    `rate` DECIMAL(5, 4) NOT NULL,
    `effective_date` TIMESTAMP(0) NOT NULL,
    `expiration_date` TIMESTAMP(0) NULL,
    `description` VARCHAR(255) NULL,
    `metadata` JSON NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `updated_at` TIMESTAMP(0) NOT NULL,

    INDEX `idx_app_jurisdiction`(`app_id`, `jurisdiction_code`),
    INDEX `idx_effective_date`(`effective_date`),
    UNIQUE INDEX `unique_tax_rate`(`app_id`, `jurisdiction_code`, `tax_type`, `effective_date`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_tax_calculations` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `invoice_id` INTEGER NULL,
    `charge_id` INTEGER NULL,
    `tax_rate_id` INTEGER NOT NULL,
    `taxable_amount` DECIMAL(10, 2) NOT NULL,
    `tax_amount` DECIMAL(10, 2) NOT NULL,
    `jurisdiction_code` VARCHAR(20) NOT NULL,
    `tax_type` VARCHAR(50) NOT NULL,
    `rate_applied` DECIMAL(5, 4) NOT NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),

    INDEX `idx_app_invoice`(`app_id`, `invoice_id`),
    INDEX `idx_app_charge`(`app_id`, `charge_id`),
    INDEX `idx_tax_rate`(`tax_rate_id`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_discount_applications` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `invoice_id` INTEGER NULL,
    `charge_id` INTEGER NULL,
    `coupon_id` INTEGER NULL,
    `customer_id` INTEGER NULL,
    `discount_type` VARCHAR(50) NOT NULL,
    `discount_amount_cents` INTEGER NOT NULL,
    `description` VARCHAR(255) NOT NULL,
    `quantity` INTEGER NULL,
    `category` VARCHAR(50) NULL,
    `product_types` JSON NULL,
    `metadata` JSON NULL,
    `applied_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `created_by` VARCHAR(255) NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),

    INDEX `billing_discount_applications_app_id_invoice_id_idx`(`app_id`, `invoice_id`),
    INDEX `billing_discount_applications_app_id_charge_id_idx`(`app_id`, `charge_id`),
    INDEX `billing_discount_applications_app_id_coupon_id_idx`(`app_id`, `coupon_id`),
    INDEX `billing_discount_applications_app_id_customer_id_idx`(`app_id`, `customer_id`),
    INDEX `billing_discount_applications_applied_at_idx`(`applied_at`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_metered_usage` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `customer_id` INTEGER NOT NULL,
    `subscription_id` INTEGER NULL,
    `metric_name` VARCHAR(100) NOT NULL,
    `quantity` DECIMAL(10, 2) NOT NULL,
    `unit_price_cents` INTEGER NOT NULL,
    `period_start` TIMESTAMP(0) NOT NULL,
    `period_end` TIMESTAMP(0) NOT NULL,
    `recorded_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),
    `billed_at` TIMESTAMP(0) NULL,

    INDEX `billing_metered_usage_app_id_customer_id_idx`(`app_id`, `customer_id`),
    INDEX `billing_metered_usage_app_id_subscription_id_idx`(`app_id`, `subscription_id`),
    INDEX `billing_metered_usage_period_start_period_end_idx`(`period_start`, `period_end`),
    INDEX `billing_metered_usage_billed_at_idx`(`billed_at`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- CreateTable
CREATE TABLE `billing_invoice_line_items` (
    `id` INTEGER NOT NULL AUTO_INCREMENT,
    `app_id` VARCHAR(50) NOT NULL,
    `invoice_id` INTEGER NOT NULL,
    `line_item_type` VARCHAR(50) NOT NULL,
    `description` VARCHAR(500) NOT NULL,
    `quantity` DECIMAL(10, 2) NOT NULL,
    `unit_price_cents` INTEGER NOT NULL,
    `amount_cents` INTEGER NOT NULL,
    `metadata` JSON NULL,
    `created_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0),

    INDEX `billing_invoice_line_items_app_id_invoice_id_idx`(`app_id`, `invoice_id`),
    INDEX `billing_invoice_line_items_line_item_type_idx`(`line_item_type`),
    PRIMARY KEY (`id`)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

-- AddForeignKey
ALTER TABLE `billing_tax_calculations` ADD CONSTRAINT `billing_tax_calculations_invoice_id_fkey` FOREIGN KEY (`invoice_id`) REFERENCES `billing_invoices`(`id`) ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_tax_calculations` ADD CONSTRAINT `billing_tax_calculations_charge_id_fkey` FOREIGN KEY (`charge_id`) REFERENCES `billing_charges`(`id`) ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_tax_calculations` ADD CONSTRAINT `billing_tax_calculations_tax_rate_id_fkey` FOREIGN KEY (`tax_rate_id`) REFERENCES `billing_tax_rates`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_discount_applications` ADD CONSTRAINT `billing_discount_applications_invoice_id_fkey` FOREIGN KEY (`invoice_id`) REFERENCES `billing_invoices`(`id`) ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_discount_applications` ADD CONSTRAINT `billing_discount_applications_charge_id_fkey` FOREIGN KEY (`charge_id`) REFERENCES `billing_charges`(`id`) ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_discount_applications` ADD CONSTRAINT `billing_discount_applications_coupon_id_fkey` FOREIGN KEY (`coupon_id`) REFERENCES `billing_coupons`(`id`) ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_discount_applications` ADD CONSTRAINT `billing_discount_applications_customer_id_fkey` FOREIGN KEY (`customer_id`) REFERENCES `billing_customers`(`id`) ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_metered_usage` ADD CONSTRAINT `billing_metered_usage_customer_id_fkey` FOREIGN KEY (`customer_id`) REFERENCES `billing_customers`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_metered_usage` ADD CONSTRAINT `billing_metered_usage_subscription_id_fkey` FOREIGN KEY (`subscription_id`) REFERENCES `billing_subscriptions`(`id`) ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE `billing_invoice_line_items` ADD CONSTRAINT `billing_invoice_line_items_invoice_id_fkey` FOREIGN KEY (`invoice_id`) REFERENCES `billing_invoices`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;