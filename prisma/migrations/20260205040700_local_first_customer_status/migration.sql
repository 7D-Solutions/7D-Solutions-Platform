-- AlterTable: Make tilled_customer_id nullable (local-first pattern)
ALTER TABLE `billing_customers` MODIFY `tilled_customer_id` VARCHAR(255) NULL;

-- AlterTable: Add status column with default 'active' for backward compatibility
ALTER TABLE `billing_customers` ADD COLUMN `status` VARCHAR(20) NOT NULL DEFAULT 'active';
