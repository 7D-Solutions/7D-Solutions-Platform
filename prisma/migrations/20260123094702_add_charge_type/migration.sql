-- AlterTable
ALTER TABLE `billing_charges`
  ADD COLUMN `charge_type` VARCHAR(50) NOT NULL DEFAULT 'one_time';

-- CreateIndex
CREATE INDEX `idx_charge_type` ON `billing_charges`(`charge_type`);
