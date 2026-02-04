-- AlterTable
ALTER TABLE `billing_charges`
  MODIFY `tilled_charge_id` VARCHAR(255) NULL,
  ADD COLUMN `reason` VARCHAR(100) NULL,
  ADD COLUMN `reference_id` VARCHAR(255) NULL,
  ADD COLUMN `service_date` TIMESTAMP(0) NULL,
  ADD COLUMN `note` TEXT NULL,
  ADD COLUMN `metadata` JSON NULL,
  ADD COLUMN `updated_at` TIMESTAMP(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0) ON UPDATE CURRENT_TIMESTAMP(0);

-- CreateIndex
CREATE INDEX `idx_reason` ON `billing_charges`(`reason`);

-- CreateIndex
CREATE INDEX `idx_service_date` ON `billing_charges`(`service_date`);

-- CreateIndex
CREATE UNIQUE INDEX `unique_app_reference_id` ON `billing_charges`(`app_id`, `reference_id`);
